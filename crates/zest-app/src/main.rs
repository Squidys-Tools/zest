//! `zest` binary: tokio runtime wiring tray → hotkey → selection → overlay → convert.
//! Installs per-user to `%LOCALAPPDATA%\\Zest` (no elevation); single instance.

use clap::Parser;
use tracing_subscriber::EnvFilter;
use zest_core::{categories_for_selection, Selection, Settings};
use zest_selection::SelectionError;

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

    // TODO(MVP-shell): enforce single instance (named mutex) before tray.
    zest_shell::tray::build()?;
    let _hotkey_id = zest_shell::hotkey::register(&settings.hotkey)?;
    let _overlay = zest_overlay::Overlay::precreate();

    if zest_shell::updater::should_check(settings.update_frequency) {
        match zest_shell::updater::check_now(env!("CARGO_PKG_VERSION")).await {
            Ok(Some(v)) => tracing::info!(version = %v, "update available — prompt on restart"),
            Ok(None) => tracing::debug!("up to date"),
            Err(e) => tracing::warn!("update check failed: {e:#}"),
        }
    }

    // TODO(MVP): hotkey event loop → on_hotkey().await; tray event loop here.
    tracing::info!("zest running (scaffold: event loop lands with shell+overlay MVP)");
    tokio::signal::ctrl_c().await?;
    Ok(())
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
