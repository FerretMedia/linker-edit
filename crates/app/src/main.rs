//! linker-edit: the GPUI window with its server, `serve` for the browser only, `mcp` for the agents' tools.
//!
//! - `linker-edit` (or `linker-edit window`): starts the server in this process and opens the window on it. A
//!   second copy on the same data folder starts nothing: it brings the first one's window forward.
//! - `linker-edit serve`: only the server, for a browser, remote access, the iPad or a computer without a display.
//! - `linker-edit --browser`: the server, and the app in the default browser. npm's launcher runs this.
//! - `linker-edit mcp`: the agents' "linker" MCP server on stdio.
//! - `linker-edit --version`: the release name.
// A release on Windows opens no console window; console.rs gives the terminal modes theirs.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod console;
mod update_state;
mod updater;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use linker_edit_server::app_update::DesktopUpdater;
use linker_edit_server::config::{AppConfig, load_config_or_exit};
use linker_edit_server::resolve::locate::find_resolve_mcp;
use linker_edit_server::{cli, index};
use linker_edit_ui::transport::{self, Auth};

const USAGE: &str = "Usage: linker-edit [window | serve | --browser | mcp | --version]";

fn main() -> anyhow::Result<()> {
    // First of all: Velopack runs the app with arguments of its own while installing, updating and uninstalling,
    // and this handles them and exits.
    velopack::VelopackApp::build().run();
    let command = std::env::args().nth(1);
    if !matches!(command.as_deref(), None | Some("window")) {
        console::attach();
    }
    match command.as_deref() {
        // `linker-edit mcp`: the agents' "linker" MCP server on stdio (agents::commands::linker_mcp_command).
        Some("mcp") => {
            use linker_edit_server::agents::linker_mcp;
            linker_mcp::run_linker_mcp(linker_mcp::catalog())
        }
        Some("--version") => {
            println!("{}", cli::version_text());
            Ok(())
        }
        // Kept for older instructions; there is nothing to set up any more.
        Some("setup") => {
            println!(
                "Nothing to set up: Linker-Edit uses the MCP server that comes with DaVinci Resolve Studio 21.1.2 and newer."
            );
            Ok(())
        }
        Some("serve" | "--serve") => serve(false),
        Some("--browser") => serve(true),
        None | Some("window") => window(),
        Some(_) => {
            eprintln!("{USAGE}");
            std::process::exit(1);
        }
    }
}

/// Logs to stderr, or with `file` (the window, which has no terminal in a release) to that file.
fn logging(file: Option<&Path>) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    let log = file.and_then(|file| {
        std::fs::create_dir_all(file.parent()?).ok()?;
        std::fs::File::create(file).ok()
    });
    let _ = match log {
        Some(log) => builder
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(log))
            .try_init(),
        None => builder.with_writer(std::io::stderr).try_init(),
    };
}

fn config() -> AppConfig {
    load_config_or_exit(find_resolve_mcp)
}

/// The data folder: the config's, ~/LinkerEdit by default.
fn data_dir(config: &AppConfig) -> PathBuf {
    config
        .config_file
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("linker-edit-server")
        .build()
        .expect("a tokio runtime for the server")
}

/// The server alone, until the process is stopped; open shows the app in the browser once it listens.
fn serve(open: bool) -> anyhow::Result<()> {
    logging(None);
    let config = config();
    runtime().block_on(async move {
        let listener = match index::listen(config.port).await {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        };
        if open {
            cli::open_browser(&index::local_url(config.port));
        }
        index::run(config, listener, None).await
    })
}

/// The window on a server of its own, started in this process; closing the window stops both.
fn window() -> anyhow::Result<()> {
    // Dev only: LINKER_EDIT_SERVER_URL=http://127.0.0.1:4521 opens the window on a server that is already
    // running. LINKER_EDIT_SERVER_TOKEN, when set, is a Linker Connect session sent as `Authorization: Bearer`;
    // without it the server must be on this computer.
    if let Ok(url) = std::env::var("LINKER_EDIT_SERVER_URL") {
        let auth = std::env::var("LINKER_EDIT_SERVER_TOKEN").map_or(Auth::Local, Auth::Bearer);
        return linker_edit_ui::run(transport::connect(&url, auth)?, || {});
    }
    // Another copy on the same data folder: it gets asked to come forward, and this one starts no server.
    if another_copy_runs() {
        let (link, _server_end) = linker_edit_ui::Link::pair();
        return linker_edit_ui::run(link, || {});
    }
    let config = config();
    let data = data_dir(&config);
    // A release has no terminal to log to; a build from source logs to the one it runs in.
    logging(
        cfg!(not(debug_assertions))
            .then(|| data.join("logs").join("linker-edit.log"))
            .as_deref(),
    );
    let port = config.port;
    // Installed by Velopack: the app updates itself. From npm or source, the server looks on npm instead.
    let updater = updater::velopack_updater(data);
    let desktop = updater
        .clone()
        .map(|updater| updater as Arc<dyn DesktopUpdater>);
    let (ready, listening) = std::sync::mpsc::channel::<Result<(), String>>();
    let start = updater.clone();
    std::thread::Builder::new()
        .name("linker-edit-server".into())
        .spawn(move || {
            runtime().block_on(async move {
                let listener = match index::listen(port).await {
                    Ok(listener) => listener,
                    Err(error) => {
                        let _ = ready.send(Err(error.to_string()));
                        return;
                    }
                };
                let _ = ready.send(Ok(()));
                if let Some(updater) = start {
                    updater.start();
                }
                if let Err(error) = index::run(config, listener, desktop).await {
                    tracing::error!("{error:#}");
                    std::process::exit(1);
                }
            });
        })?;
    match listening.recv() {
        Ok(Ok(())) => {}
        Ok(Err(message)) => {
            tracing::error!("{message}");
            eprintln!("{message}");
            std::process::exit(1);
        }
        Err(_) => std::process::exit(1),
    }
    // Closing the window with an update downloaded installs it, as the Electron app did.
    linker_edit_ui::run(
        transport::connect(&index::local_url(port), Auth::Local)?,
        move || {
            if let Some(updater) = updater {
                updater.quitting();
            }
        },
    )
}

/// Whether a copy of the window already runs on this data folder: crates/ui's single_instance listens on a local
/// socket named after it, and is asked to come forward by linker_edit_ui::run. Checked here first so a second
/// copy doesn't start a server.
fn another_copy_runs() -> bool {
    use interprocess::local_socket::{GenericNamespaced, Stream, prelude::*};
    linker_edit_ui::single_instance::socket_name()
        .as_str()
        .to_ns_name::<GenericNamespaced>()
        .is_ok_and(|name| Stream::connect(name).is_ok())
}
