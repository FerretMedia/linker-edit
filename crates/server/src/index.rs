// The server put together: the app with the real Resolve, agents, history, library and accounts, and its HTTP side.
// The port of server/index.ts; cli.rs and the linker-edit binary start it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Context as _;
use linker_edit_protocol::release_name::release_name;
use linker_edit_protocol::{ComputerSetup, Provider, RemoteAccess, SetupFolders};
use tokio::net::TcpListener;

use crate::app::{App, AppOptions, SessionRequest, ToolOutput, create_app};
use crate::app_update::{AppUpdates, DesktopUpdater};
use crate::collections::store::file_collection_store;
use crate::config::AppConfig;
use crate::connect::cloudflared::Cloudflared;
use crate::connect::connect::{Connect, ConnectOptions};
use crate::connect::sessions::Sessions;
use crate::history::history::{HistoryOptions, ResolveSnapshots, create_history};
use crate::http::{ConnectSide, HttpOptions, Web, default_probe, router};
use crate::library::folders::file_library_store;
use crate::library::scan::ffmpeg_probe;
use crate::library::{Library, LibraryOptions};
use crate::linker::{Linker, LinkerOptions};
use crate::preview::preview::create_preview_renderer;
use crate::projects::store::file_project_store;
use crate::providers::claude::{ClaudeSessionOptions, create_claude_session};
use crate::providers::codex::create_codex_session;
use crate::providers::replay::{STEP_MS, replay_query};
use crate::providers::types::{Policy, ProviderSession};
use crate::remote::{RemoteAccessOptions, RemoteAccessService};
use crate::resolve::connection::{ConnectionOptions, ResolveConnection, create_resolve_connection};
use crate::resolve::run::{ToolContext, run_resolve_tool};
use crate::resolve::versions::{apply_timeline, backup_project};
use crate::resolve_projects::create_resolve_projects;
use crate::resolve_status::check_resolve;
use crate::rules::prepare_workspace;
use crate::settings::file_settings;
use crate::study::{StudyOptions, StudyService};
use crate::transcribe::tools::{find_ffmpeg, tools_ready};
use crate::transcribe::{DownloadTools, Transcriber, TranscriberOptions};
use crate::updates::{VERSION, is_newer, latest_version};

/// This server's address on this computer.
pub fn local_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// Takes the port on 127.0.0.1; Err says what to do when another server has it.
pub async fn listen(port: u16) -> anyhow::Result<TcpListener> {
    match TcpListener::bind(("127.0.0.1", port)).await {
        Ok(listener) => Ok(listener),
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => anyhow::bail!(
            "Port {port} is already in use. Is another Linker-Edit server still running?"
        ),
        Err(error) => Err(error.into()),
    }
}

/// LINKER_EDIT_REPLAY: a recorded Claude turn (JSONL) to replay instead of calling Claude, for free turns in UI work.
pub fn replay_file() -> Option<PathBuf> {
    std::env::var_os("LINKER_EDIT_REPLAY")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Runs the server on listener until the process ends. Prints where it is, then starts what runs beside it.
/// `updater` is the installed desktop app's own (Velopack, in crates/app); without one, as from npm or a build
/// from source, the server looks on npm for a newer version instead.
pub async fn run(
    config: AppConfig,
    listener: TcpListener,
    updater: Option<Arc<dyn DesktopUpdater>>,
) -> anyhow::Result<()> {
    let replay = replay_file();
    if let Some(replay) = &replay {
        println!("Replaying {} instead of calling Claude", replay.display());
    }

    // The one connection to Resolve, through the MCP server Resolve Studio comes with. An empty resolveMcp in the
    // config means this computer has none, as in tests.
    let resolve_command = config
        .resolve_mcp
        .command
        .clone()
        .filter(|command| !command.as_os_str().is_empty());
    let resolve: Arc<dyn ResolveConnection> =
        Arc::new(create_resolve_connection(ConnectionOptions {
            command: resolve_command.clone(),
            ..ConnectionOptions::default()
        }));

    let remote = Arc::new(RemoteAccessService::new(RemoteAccessOptions::new(
        config.port,
        config.tailscale_port,
    )));
    // The browser comes back to this server after signing in at Linker.
    let linker = Arc::new(Linker::new(LinkerOptions::new(
        &config.linker,
        format!("{}/api/linker/callback", local_url(config.port)),
    )));
    // Linker Connect: up to 3 of your edit computers at their own addresses at Linker, through Cloudflare Tunnels.
    let sessions = Arc::new(Sessions::new(config.connect.sessions_file.clone()));
    let connect = Connect::new(ConnectOptions::new(
        config.connect.file.clone(),
        config.port,
        linker.clone(),
        config.linker.app_url.clone(),
        sessions.clone(),
        Arc::new(Cloudflared {
            dir: config.tools_dir.clone(),
            override_file: config.connect.cloudflared.clone(),
        }),
    ));

    let (tools_dir, whisper_cli) = (config.tools_dir.clone(), config.whisper_cli.clone());
    let transcriber = Arc::new(Transcriber::new(TranscriberOptions {
        dir: config.transcripts_dir.clone(),
        prepare: Arc::new(DownloadTools {
            dir: tools_dir.clone(),
            whisper_cli: whisper_cli.clone(),
        }),
        ready: Some(Arc::new(move || {
            tools_ready(&tools_dir, whisper_cli.as_deref())
        })),
        transcribe_file: None,
    }));

    let mut options = AppOptions::new(
        VERSION,
        Arc::new(file_project_store(
            &config.projects_dir,
            &config.workspace_dir,
        )?),
        create_session(&config, replay.clone()),
        {
            let resolve = resolve.clone();
            Arc::new(move || {
                let resolve = resolve.clone();
                Box::pin(async move { check_resolve(resolve.as_ref()).await })
            })
        },
    );
    // Workspaces and series live next to the config: ~/LinkerEdit/collections.json and collections/.
    options.collections = Some(Arc::new(file_collection_store(
        config.config_file.parent().unwrap_or(Path::new(".")),
    )));
    // A replayed turn never touched Resolve, so there is nothing to save or back up.
    if replay.is_none() {
        let resolve_for_history = resolve.clone();
        options.create_history = Some(Arc::new(move |dir, check| {
            Arc::new(create_history(HistoryOptions {
                dir: dir.to_path_buf(),
                read_snapshot: Arc::new(ResolveSnapshots(resolve_for_history.clone())),
                check_project: Some(check),
            }))
        }));
        let resolve_for_backup = resolve.clone();
        options.backup_project = Some(Arc::new(move |dir| {
            let resolve = resolve_for_backup.clone();
            Box::pin(async move {
                let now = chrono::Local::now().naive_local();
                let file = backup_project(resolve.as_ref(), &dir, now).await?;
                Ok(file.to_string_lossy().into_owned())
            })
        }));
    }
    options.preview = Some(Arc::new(create_preview_renderer(
        config.preview_dir.clone(),
        resolve.clone(),
        Duration::from_millis(500),
    )));
    options.resolve_projects = Some(Arc::new(create_resolve_projects(
        resolve.clone(),
        Duration::from_secs(3),
    )));
    {
        let resolve = resolve.clone();
        options.resolve_tools = Some(Arc::new(move |tool, input, dir, signal| {
            let resolve = resolve.clone();
            Box::pin(async move {
                let context = ToolContext {
                    signal,
                    ..ToolContext::new(dir)
                };
                let result = run_resolve_tool(resolve, &tool, input, &context).await?;
                Ok(ToolOutput {
                    text: result.text,
                    images: (!result.images.is_empty()).then(|| {
                        result
                            .images
                            .iter()
                            .map(|image| image.to_string_lossy().into_owned())
                            .collect()
                    }),
                })
            })
        }));
    }
    {
        let resolve = resolve.clone();
        options.apply_timeline = Some(Arc::new(move |dir, timeline_id| {
            let resolve = resolve.clone();
            Box::pin(async move {
                let now = chrono::Local::now().naive_local();
                apply_timeline(resolve.as_ref(), &dir, &timeline_id, now).await
            })
        }));
    }
    // Codex reads the project's rules from AGENTS.md, written before each turn.
    options.prepare_folder = Some(Arc::new(|dir, rules| {
        prepare_workspace(dir, rules).map_err(Into::into)
    }));
    options.transcriber = Some(transcriber.clone());
    // Measurements and contact sheets of example videos, next to the library's cache.
    options.study = Some(Arc::new(StudyService::new(StudyOptions {
        dir: config
            .library_cache_dir
            .parent()
            .unwrap_or(Path::new("."))
            .join("study"),
        ffmpeg: Arc::new(find_ffmpeg),
        transcribe: transcriber.clone(),
        measure: None,
    })));
    options.remote = Some(remote.clone());
    options.linker = Some(linker.clone());
    options.connect = Some(connect.clone());
    options.settings = Some(Arc::new(file_settings(config.settings_file.clone())));
    options.setup = Some(ComputerSetup {
        config_file: config.config_file.to_string_lossy().into_owned(),
        folders: SetupFolders {
            projects: config.projects_dir.to_string_lossy().into_owned(),
            previews: config.preview_dir.to_string_lossy().into_owned(),
            transcripts: config.transcripts_dir.to_string_lossy().into_owned(),
            tools: config.tools_dir.to_string_lossy().into_owned(),
        },
        resolve_mcp: resolve_command.map(|command| command.to_string_lossy().into_owned()),
    });
    options.library = Some(Arc::new(Library::new(LibraryOptions {
        store: Arc::new(file_library_store(config.library_file.clone())),
        cache_dir: config.library_cache_dir.clone(),
        probe: Arc::new(ffmpeg_probe(Arc::new(find_ffmpeg))),
        on_error: None,
    })));
    let has_updater = updater.is_some();
    // Restart to update waits until nothing is busy with Resolve; the app it asks is made just below.
    let made: Arc<OnceLock<App>> = Arc::default();
    if let Some(updater) = updater {
        let made = made.clone();
        let busy = Arc::new(move || made.get().is_some_and(App::busy));
        options.app_updates = Some(AppUpdates::with_updater(updater, busy));
    }
    let app = create_app(options);
    let _ = made.set(app.clone());

    let routes = router(HttpOptions {
        app: app.clone(),
        web: Web::Embedded,
        api_token: config.api_token.clone(),
        remote: remote.clone(),
        linker: Some(linker.clone()),
        connect: Some(ConnectSide {
            routes: connect.clone(),
            sessions,
            probe: default_probe(),
        }),
    });

    println!(
        "Linker-Edit {} on {}",
        release_name(VERSION),
        local_url(config.port)
    );
    println!("Projects: {}", config.projects_dir.display());
    // Tailscale keeps Serve on across restarts, so remote access may already be on.
    tokio::spawn(async move {
        remote.refresh().await;
        if let RemoteAccess::On { url, owner } = remote.status() {
            println!("Remote access: {url} (for {owner})");
        }
    });
    tokio::spawn(app.refresh_resolve());
    app.prepare_transcription();
    // Picks up files added to library folders while Linker-Edit was closed; unchanged files aren't measured again.
    tokio::spawn(app.scan_library());
    tokio::spawn(async move { linker.refresh().await });
    // Linker Connect stays on across restarts, like Tailscale's remote access.
    connect.start();
    {
        let app = app.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(Duration::from_secs(15));
            every.tick().await;
            loop {
                every.tick().await;
                if !app.busy() {
                    app.refresh_resolve().await;
                }
            }
        });
    }
    // A checkout runs as 0.0.0 and has nothing to update to. The installed desktop app updates itself instead.
    if VERSION != "0.0.0" && !has_updater {
        let app = app.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(Duration::from_secs(24 * 60 * 60));
            loop {
                every.tick().await;
                let latest = latest_version().await;
                app.set_update(latest.filter(|latest| is_newer(latest, VERSION)));
            }
        });
    }
    axum::serve(listener, routes)
        .await
        .context("The server stopped")
}

/// The agent sessions: Codex, or Claude (a replayed turn under LINKER_EDIT_REPLAY), in the project's folder.
fn create_session(config: &AppConfig, replay: Option<PathBuf>) -> crate::app::CreateSession {
    let config = config.clone();
    Arc::new(move |request: SessionRequest| -> Arc<dyn ProviderSession> {
        let project_config = AppConfig {
            workspace_dir: request.dir.clone(),
            ..config.clone()
        };
        match request.provider {
            Provider::Codex => Arc::new(create_codex_session(
                project_config,
                request.approval_mode,
                request.resume_id,
            )),
            Provider::Claude => {
                let rules = request.rules.clone();
                let mut options = ClaudeSessionOptions::new(move || rules());
                options.resume_id = request.resume_id;
                // A fresh replay per session, as each starts its recorded turn from the top.
                if let Some(file) = &replay {
                    options.run_query = Arc::new(replay_query(file.clone(), STEP_MS));
                }
                Arc::new(create_claude_session(
                    project_config,
                    request.approval_mode,
                    Arc::new(Policy),
                    options,
                ))
            }
        }
    })
}
