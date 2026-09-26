//! `zest` binary: tokio runtime wiring tray → hotkey → selection → overlay → convert.
//! Installs per-user to `%LOCALAPPDATA%\\Zest` (no elevation); single instance.

use std::sync::atomic::{AtomicBool, Ordering};

use clap::Parser;
use tracing_subscriber::EnvFilter;
use zest_convert::Job;
use zest_core::{
    convert_menu_for_selection, menu_for_selection, MenuAction, MenuNode, Selection, Settings,
};
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

    let (overlay, mut choices) = Overlay::precreate()?;
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
    // The selection the visible menu was built from. The overlay never takes
    // focus, so the Explorer selection cannot change while it is up.
    let mut shown: Option<Selection> = None;
    loop {
        tokio::select! {
            action = tray_rx.recv() => match action {
                Some(TrayAction::Show) => show_overlay(&overlay, &mut shown),
                Some(TrayAction::Settings) => open_settings(None),
                Some(TrayAction::Quit) | None => break,
            },
            action = async {
                match hotkey_listener.as_mut() {
                    Some(listener) => listener.recv().await,
                    None => std::future::pending().await,
                }
            } => match action {
                Some(zest_shell::hotkey::HotkeyAction::OpenMenu) => show_overlay(&overlay, &mut shown),
                Some(zest_shell::hotkey::HotkeyAction::OpenConvert) => show_convert_overlay(&overlay, &mut shown),
                None => {
                    tracing::error!("global hotkey listener stopped; tray actions remain available");
                    open_settings(Some(
                        "The global hotkey listener stopped unexpectedly. Tray actions still work; restart Zest to try again.".into(),
                    ));
                    hotkey_listener = None;
                }
            },
            choice = choices.recv() => match choice {
                Some(choice) => run_choice(choice, shown.take()),
                None => {
                    tracing::error!("overlay stopped; tray actions remain available");
                    open_settings(Some(
                        "The overlay stopped unexpectedly. Tray actions still work; restart Zest to try again.".into(),
                    ));
                }
            },
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    tracing::info!("zest exiting");
    Ok(())
}

/// Tray Show / hotkey path: the two-ring menu for the live selection.
fn show_overlay(overlay: &Overlay, shown: &mut Option<Selection>) {
    let (selection, menu) = match resolve_selection() {
        Ok(selection) => {
            let menu = menu_for_selection(&selection);
            (Some(selection), menu)
        }
        // Selection unknown here; picking a leaf re-reads it.
        Err(()) => (None, fallback_menu()),
    };
    *shown = if show_menu(overlay, &menu) {
        selection
    } else {
        None
    };
}

fn show_convert_overlay(overlay: &Overlay, shown: &mut Option<Selection>) {
    if let Ok(selection) = resolve_selection() {
        let menu = convert_menu_for_selection(&selection);
        if menu.is_empty() {
            tracing::info!("selection has no Convert targets");
            *shown = None;
            return;
        }
        *shown = show_menu(overlay, &menu).then_some(selection);
    }
}

fn resolve_selection() -> Result<Selection, ()> {
    match zest_selection::resolve() {
        Ok(selection) => Ok(selection),
        Err(SelectionError::VirtualFolder) => {
            tracing::info!("selection is a virtual folder; no menu to show");
            Err(())
        }
        Err(error) => {
            tracing::debug!("selection resolve failed ({error}); showing fallback ring");
            Err(())
        }
    }
}

/// Menu shown when the selection cannot be read: the categories and targets a
/// PNG would get, so the menu still tells the truth about what Zest can do.
fn fallback_menu() -> Vec<MenuNode> {
    menu_for_selection(&Selection::new(vec![std::path::PathBuf::from(
        "selection.png",
    )]))
}

/// Show the menu; reports whether it is now on screen.
fn show_menu(overlay: &Overlay, menu: &[MenuNode]) -> bool {
    if menu.is_empty() {
        tracing::info!("nothing to convert for this selection");
        return false;
    }
    if let Err(error) = overlay.show(menu) {
        tracing::warn!("overlay show failed: {error:#}");
        return false;
    }
    true
}

/// Run the picked action for every selected file. Runs off the event loop so a
/// slow conversion never stalls the hotkeys. `selection` is the snapshot the
/// visible menu was built from; without one the selection is re-read.
fn run_choice(choice: MenuAction, selection: Option<Selection>) {
    tokio::spawn(async move {
        let selection = match selection.filter(|selection| !selection.is_empty()) {
            Some(selection) => selection,
            None => match resolve_now() {
                Ok(selection) => selection,
                Err(()) => return,
            },
        };
        run_action(&choice, &selection).await;
    });
}

fn resolve_now() -> Result<Selection, ()> {
    match zest_selection::resolve() {
        Ok(selection) if !selection.is_empty() => Ok(selection),
        Ok(_) => {
            tracing::warn!("nothing is selected; nothing to run");
            Err(())
        }
        Err(SelectionError::VirtualFolder) => {
            tracing::warn!("selection is a virtual folder; nothing to run");
            Err(())
        }
        Err(error) => {
            tracing::warn!("selection resolve failed ({error}); nothing to run");
            Err(())
        }
    }
}

async fn run_action(choice: &MenuAction, selection: &Selection) {
    let settings = Settings::load();
    for input in &selection.files {
        let mut job = match choice {
            MenuAction::Convert { ext } => Job::new(input, ext),
            MenuAction::Archive { ext } => Job::archive(input, ext),
            MenuAction::Extract => Job::extract(input),
        };
        tracing::info!(
            input = %input.display(),
            operation = ?job.operation,
            ext = %job.output_ext,
            kind = ?zest_core::file_kind::classify_path(input),
            "running menu action"
        );
        match zest_convert::dispatch(&mut job, &settings).await {
            Ok(output) => tracing::info!(output = %output.display(), "done"),
            Err(error) => tracing::warn!(input = %input.display(), "failed: {error}"),
        }
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
    println!("files: {:?}", sel.files);
    print_ring(1, &menu_for_selection(sel));
    println!("shift+c ring:");
    print_ring(1, &convert_menu_for_selection(sel));
    Ok(())
}

fn print_ring(depth: usize, nodes: &[MenuNode]) {
    for node in nodes {
        let indent = "  ".repeat(depth);
        match &node.action {
            Some(action) => println!("{indent}{} -> {action:?}", node.label),
            None => println!("{indent}{}", node.label),
        }
        print_ring(depth + 1, &node.children);
    }
}
