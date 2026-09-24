//! `zest` binary: tokio runtime wiring tray → hotkey → selection → overlay → convert.
//! Installs per-user to `%LOCALAPPDATA%\\Zest` (no elevation); single instance.

use std::sync::atomic::{AtomicBool, Ordering};

use clap::Parser;
use tracing_subscriber::EnvFilter;
use zest_core::menu::ActionCategory;
use zest_core::{categories_for_selection, Selection, Settings};
use zest_overlay::Overlay;
use zest_selection::SelectionError;
use zest_shell::tray::TrayAction;

#[derive(Debug, Parser)]
#[command(name = "zest", about = "Radial file converter (scaffold)")]
struct Cli {
    /// Print resolved Explorer selection and the menu it would show, then exit.
    #[arg(long)]
    check_selection: bool,
    /// Open the settings window, then exit.
    #[arg(long)]
    settings: bool,
}

/// At most one settings window at a time (the tray can fire Settings repeatedly).
static SETTINGS_OPEN: AtomicBool = AtomicBool::new(false);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let settings = Settings::load();
    tracing::info!(hotkey = %settings.hotkey, "settings loaded");

    if cli.settings {
        return zest_settings::run(settings);
    }

    if cli.check_selection {
        return check_selection().await;
    }

    // Single instance before tray/hotkey: a second launch no-ops (SQU-26).
    let _instance = match zest_shell::instance::acquire()? {
        Some(guard) => guard,
        None => {
            tracing::info!("another Zest instance is already running; exiting");
            return Ok(());
        }
    };

    let overlay = Overlay::precreate()?;
    let mut tray_rx = zest_shell::tray::build()?;
    let (mut hotkey_listener, mut hotkey_notice) = match zest_shell::hotkey::register(
        &settings.hotkey,
    ) {
        Ok(listener) => (Some(listener), None),
        Err(error) if !zest_shell::hotkey::uses_default_hotkey(&settings.hotkey) => {
            tracing::warn!(
                hotkey = %settings.hotkey,
                "configured menu hotkey failed to register: {error:#}; trying the default"
            );
            match zest_shell::hotkey::register(zest_shell::hotkey::DEFAULT_HOTKEY) {
                Ok(listener) => (
                    Some(listener),
                    Some(format!(
                        "Menu shortcut '{}' could not be registered: {error:#}. Using {} instead. Correct it below, save, and restart Zest.",
                        settings.hotkey,
                        zest_shell::hotkey::DEFAULT_HOTKEY
                    )),
                ),
                Err(default_error) => {
                    tracing::error!("default menu hotkey failed to register: {default_error:#}");
                    (
                        None,
                        Some(format!(
                            "Neither menu shortcut '{}' nor the default '{}' could be registered: {error:#}; {default_error:#}. Tray actions remain available. Change the shortcut below and restart Zest.",
                            settings.hotkey,
                            zest_shell::hotkey::DEFAULT_HOTKEY
                        )),
                    )
                }
            }
        }
        Err(error) => {
            tracing::error!(hotkey = %settings.hotkey, "default menu hotkey failed to register: {error:#}");
            (
                None,
                Some(format!(
                    "Default menu shortcut '{}' could not be registered: {error:#}. Tray actions remain available. Close the app using this shortcut or select another shortcut below, then restart Zest.",
                    settings.hotkey
                )),
            )
        }
    };
    if let Some(listener) = &hotkey_listener {
        if let Some(error) = listener.convert_unavailable_reason() {
            tracing::warn!(
                convert_hotkey = zest_shell::hotkey::DEFAULT_CONVERT_HOTKEY,
                "Convert hotkey could not be registered: {error}"
            );
            let notice = format!(
                "Convert shortcut '{}' could not be registered: {error}. Your menu shortcut still works; close the other app using {} to restore direct Convert access.",
                zest_shell::hotkey::DEFAULT_CONVERT_HOTKEY,
                zest_shell::hotkey::DEFAULT_CONVERT_HOTKEY
            );
            if let Some(startup_notice) = &mut hotkey_notice {
                startup_notice.push(' ');
                startup_notice.push_str(&notice);
            } else {
                hotkey_notice = Some(notice);
            }
        }
    }
    if let Some(notice) = hotkey_notice {
        open_settings(Some(notice));
    }

    if zest_shell::updater::should_check(settings.update_frequency) {
        match zest_shell::updater::check_now(env!("CARGO_PKG_VERSION")).await {
            Ok(Some(v)) => tracing::info!(version = %v, "update available — prompt on restart"),
            Ok(None) => tracing::debug!("up to date"),
            Err(e) => tracing::warn!("update check failed: {e:#}"),
        }
    }

    tracing::info!("zest running");
    loop {
        tokio::select! {
            action = tray_rx.recv() => match action {
                Some(TrayAction::Show) => show_overlay(&overlay),
                Some(TrayAction::Settings) => open_settings(None),
                Some(TrayAction::Quit) | None => break,
            },
            action = async {
                match hotkey_listener.as_mut() {
                    Some(listener) => listener.recv().await,
                    None => std::future::pending().await,
                }
            } => match action {
                Some(zest_shell::hotkey::HotkeyAction::OpenMenu) => show_overlay(&overlay),
                Some(zest_shell::hotkey::HotkeyAction::OpenConvert) => show_convert_overlay(&overlay),
                None => break,
            },
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    tracing::info!("zest exiting");
    Ok(())
}

/// Tray Show / (future) hotkey path: ring-1 labels for the live selection.
fn show_overlay(overlay: &Overlay) {
    let labels: Vec<String> = match zest_selection::resolve() {
        Ok(sel) => {
            let cats = categories_for_selection(&sel);
            zest_overlay::ring_labels(&cats)
        }
        Err(SelectionError::VirtualFolder) => {
            tracing::info!("selection is a virtual folder; no menu to show");
            return;
        }
        Err(e) => {
            tracing::debug!("selection resolve failed ({e}); showing fallback ring");
            Vec::new()
        }
    };
    let labels = if labels.is_empty() {
        vec!["Convert".to_string(), "Archive".to_string()]
    } else {
        labels
    };
    if let Err(error) = overlay.show(&labels) {
        tracing::warn!("overlay show failed: {error:#}");
    }
}

fn show_convert_overlay(overlay: &Overlay) {
    let selection = match zest_selection::resolve() {
        Ok(selection) => selection,
        Err(SelectionError::VirtualFolder) => {
            tracing::info!("selection is a virtual folder; no Convert menu to show");
            return;
        }
        Err(error) => {
            tracing::debug!("selection resolve failed ({error}); no Convert menu to show");
            return;
        }
    };

    if !categories_for_selection(&selection).contains(&ActionCategory::Convert) {
        tracing::info!("selection has no Convert action");
        return;
    }

    let Some(path) = selection.files.first() else {
        return;
    };
    let kind = zest_core::file_kind::classify_path(path);
    let labels: Vec<String> = zest_core::convert_targets(kind)
        .iter()
        .map(|target| (*target).to_string())
        .collect();
    if labels.is_empty() {
        tracing::info!(?kind, "selection has no Convert targets");
        return;
    }
    if let Err(error) = overlay.show(&labels) {
        tracing::warn!("Convert overlay show failed: {error:#}");
    }
}

/// Open the settings window on its own thread (eframe blocks) without
/// stacking duplicates when Settings is clicked twice.
fn open_settings(startup_notice: Option<String>) {
    if SETTINGS_OPEN.swap(true, Ordering::SeqCst) {
        tracing::info!("settings window already open");
        return;
    }
    std::thread::spawn(move || {
        let settings = Settings::load();
        if let Err(e) = zest_settings::run_with_notice(settings, startup_notice) {
            tracing::warn!("settings window: {e:#}");
        }
        SETTINGS_OPEN.store(false, Ordering::SeqCst);
    });
}

async fn check_selection() -> anyhow::Result<()> {
    match zest_selection::resolve() {
        Ok(sel) => print_menu(&sel),
        Err(SelectionError::VirtualFolder) => {
            tracing::info!("selection is a virtual folder; no menu to show");
            Ok(())
        }
        Err(e) => {
            tracing::warn!("{e}; showing mock example (single PNG)");
            print_menu(&Selection::new(vec!["photo.png".into()]))?;
            tracing::info!("resolve error was: {e}");
            Ok(())
        }
    }
}

fn print_menu(sel: &Selection) -> anyhow::Result<()> {
    let cats = categories_for_selection(sel);
    println!("files: {:?}", sel.files);
    println!("ring 1: {cats:?}");
    for c in &cats {
        if matches!(c, zest_core::menu::ActionCategory::Convert) {
            let kind = sel
                .files
                .first()
                .map(|p| zest_core::file_kind::classify_path(p));
            if let Some(k) = kind {
                println!("ring 2 (convert): {:?}", zest_core::convert_targets(k));
            }
        }
    }
    Ok(())
}
