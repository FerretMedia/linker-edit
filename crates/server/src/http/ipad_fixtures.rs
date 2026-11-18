// Records what the server really sends over /ws, for the iPad app's tests (apps/ipad): its Swift models must
// decode every file, so a protocol change that breaks the iPad shows up there. Run it again after changing
// crates/protocol, then push: the iPad CI decodes the new files. It does nothing without RECORD_IPAD_FIXTURES=1.
//
//   RECORD_IPAD_FIXTURES=1 cargo test -p linker-edit-server ipad_fixtures

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use linker_edit_protocol::{
    AppState, ApprovalMode, ClientMessage, LocalLink, PreviewState, Provider, ResolveDatabase,
    ResolveLink, ResolveLinkKind, ResolveStatus, SidebarClientMessage, Tone, ToolStatus,
};
use regex::{Captures, Regex};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest};
use tokio_util::sync::CancellationToken;

use super::{HttpOptions, Web, router};
use crate::app::testing::{apply_server_message, item_id};
use crate::app::{AppOptions, SessionRequest, create_app};
use crate::preview::preview::{OnPreviewEvent, PreviewEvent, PreviewJob, PreviewRenderer};
use crate::projects::store::{LoadedProject, NEW_THREAD_SIDEBAR, SavedProject, SavedThread};
use crate::projects::testing::{memory_project_store, workspace_project};
use crate::providers::types::{ItemKind, ItemPatch, ProviderSession, TurnSink};
use crate::resolve::versions::TimelineRef;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/ipad/Packages/LinkerEditKit/Tests/Fixtures")
}

fn open() -> ResolveStatus {
    ResolveStatus::Connected {
        product: "DaVinci Resolve Studio".into(),
        version: "21.1.2".into(),
        project: Some("Interview".into()),
        project_id: Some("resolve-interview".into()),
        database: Some(ResolveDatabase {
            r#type: "Disk".into(),
            name: "Local Database".into(),
            host: None,
        }),
        timeline: Some("Cut".into()),
    }
}

/// What the iPad sends, typed against the protocol here and compared with what the Swift models encode.
fn client_messages() -> Vec<(&'static str, ClientMessage)> {
    let send = |text: &str| ClientMessage::Send {
        text: text.into(),
        compact_first: None,
    };
    let approve = |allow| ClientMessage::Approve {
        id: "request-1".into(),
        allow,
    };
    let render = |timeline_id: Option<&str>| ClientMessage::RenderPreview {
        timeline_id: timeline_id.map(str::to_string),
    };
    vec![
        ("send", send("Cut the silences")),
        ("stop", ClientMessage::Stop),
        ("approve", approve(true)),
        ("deny", approve(false)),
        (
            "select_thread",
            ClientMessage::SelectThread {
                thread_id: "thread-1".into(),
            },
        ),
        (
            "new_thread",
            ClientMessage::NewThread {
                project_id: "project-1".into(),
            },
        ),
        (
            "set_approval_mode",
            ClientMessage::SetApprovalMode {
                mode: ApprovalMode::Ask,
            },
        ),
        (
            "set_provider",
            ClientMessage::SetProvider {
                provider: Provider::Codex,
            },
        ),
        ("refresh_resolve", ClientMessage::RefreshResolve),
        ("render_preview", render(None)),
        ("render_preview_timeline", render(Some("timeline-1"))),
        ("stop_preview", ClientMessage::StopPreview),
        (
            "mark_read",
            ClientMessage::Sidebar(SidebarClientMessage::MarkRead {
                thread_id: "thread-1".into(),
                read: true,
            }),
        ),
    ]
}

static UUID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").unwrap()
});
static ISO_TIME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z").unwrap());

/// Ids and times change on every run, and the computer's name is the developer's: fixed values keep diffs small.
fn stable(raw: &str, ids: &mut HashMap<String, String>) -> String {
    let hostname = gethostname::gethostname().to_string_lossy().into_owned();
    let raw = raw.replace(&hostname, "edit-pc");
    let raw = UUID.replace_all(&raw, |id: &Captures| {
        let next = ids.len() + 1;
        ids.entry(id[0].to_string())
            .or_insert_with(|| format!("00000000-0000-4000-8000-{next:012}"))
            .clone()
    });
    ISO_TIME
        .replace_all(&raw, "2026-11-17T09:00:00.000Z")
        .into_owned()
}

/// The scripted turn: streamed text, a tool, an approval the user gives, a notice.
struct Turn {
    asked: Arc<AtomicBool>,
}

fn item(id: &str, kind: ItemKind) -> ItemPatch {
    ItemPatch {
        id: id.into(),
        kind: Some(kind),
        ..ItemPatch::default()
    }
}

fn assistant(id: &str, text: &str) -> ItemPatch {
    ItemPatch {
        text: Some(text.into()),
        ..item(id, ItemKind::Assistant)
    }
}

fn tool(id: &str, name: &str, input: Value, status: ToolStatus, output: Option<&str>) -> ItemPatch {
    ItemPatch {
        name: Some(name.into()),
        input: Some(input),
        status: Some(status),
        output: output.map(str::to_string),
        ..item(id, ItemKind::Tool)
    }
}

#[async_trait]
impl ProviderSession for Turn {
    async fn send(
        &self,
        _: &str,
        sink: Arc<dyn TurnSink>,
        _: CancellationToken,
    ) -> anyhow::Result<()> {
        sink.upsert(assistant("a1", "Looking at"));
        sink.upsert(assistant("a1", "Looking at the timeline first."));
        let list = "mcp__linker__list_timelines";
        sink.upsert(tool("t1", list, json!({}), ToolStatus::Running, None));
        sink.upsert(tool(
            "t1",
            list,
            json!({}),
            ToolStatus::Done,
            Some(r#"["Cut"]"#),
        ));
        let create = "mcp__linker__create_timeline";
        let input = json!({ "name": "AI - Cut", "from": "Cut" });
        self.asked.store(true, Ordering::SeqCst);
        let allowed = sink.request_approval(create, &input).await;
        let status = if allowed {
            ToolStatus::Done
        } else {
            ToolStatus::Error
        };
        sink.upsert(tool(
            "t2",
            create,
            input,
            status,
            Some("Created AI - Cut with 41 clips."),
        ));
        sink.notice(
            "Backed up the project before the first change.".into(),
            Tone::Info,
        );
        sink.upsert(assistant(
            "a2",
            "Done: **AI - Cut** has the 12 long pauses taken out.",
        ));
        Ok(())
    }

    fn resume_id(&self) -> Option<String> {
        None
    }
}

/// Keeps each render's callback, for the test to report its progress.
#[derive(Default)]
struct Renders(Mutex<Vec<OnPreviewEvent>>);

impl PreviewRenderer for Renders {
    fn render(&self, _: Option<String>, on_event: OnPreviewEvent) -> PreviewJob {
        self.0.lock().unwrap().push(on_event);
        PreviewJob::new(CancellationToken::new())
    }

    fn file(&self, _: &str) -> Option<PathBuf> {
        None
    }
}

fn project(id: &str, dir: Option<&str>, name: &str, resolve_id: &str) -> SavedProject {
    let base = workspace_project();
    SavedProject {
        id: id.into(),
        dir: dir.map(PathBuf::from).unwrap_or(base.dir.clone()),
        name: Some(name.into()),
        link: Some(ResolveLink {
            kind: ResolveLinkKind::Local,
            name: name.into(),
            project_id: Some(resolve_id.into()),
        }),
        local: LocalLink::default(),
        ..base
    }
}

fn thread(id: &str, title: &str, at: &str) -> SavedThread {
    SavedThread {
        id: id.into(),
        title: title.into(),
        provider: Provider::Claude,
        resume_id: None,
        items: Vec::new(),
        context: None,
        created_at: at.into(),
        updated_at: at.into(),
        archived_at: None,
        sidebar: NEW_THREAD_SIDEBAR,
    }
}

/// What the socket received, as raw text and as the UI's state.
#[derive(Default)]
struct Received {
    raw: Vec<String>,
    state: Option<AppState>,
}

impl Received {
    fn types(&self) -> Vec<String> {
        self.raw
            .iter()
            .map(|message| message_type(message))
            .collect()
    }
}

fn message_type(raw: &str) -> String {
    let message: Value = serde_json::from_str(raw).unwrap();
    message["type"].as_str().unwrap().to_string()
}

type Writer = futures::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    tungstenite::Message,
>;

/// The iPad's side of /ws: sends what it sends, and keeps state as its Store does.
struct Socket {
    writer: Writer,
    received: Arc<Mutex<Received>>,
    changed: Arc<watch::Sender<u64>>,
    reading: tokio::task::JoinHandle<()>,
}

impl Socket {
    async fn open(port: u16) -> Socket {
        let mut request = format!("ws://127.0.0.1:{port}/ws")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "origin",
            format!("http://127.0.0.1:{port}").parse().unwrap(),
        );
        let (socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let (writer, mut reader) = socket.split();
        let received = Arc::new(Mutex::new(Received::default()));
        let changed = Arc::new(watch::channel(0u64).0);
        let reading = {
            let received = received.clone();
            let changed = changed.clone();
            tokio::spawn(async move {
                while let Some(Ok(message)) = reader.next().await {
                    let tungstenite::Message::Text(text) = message else {
                        continue;
                    };
                    let text = text.to_string();
                    let parsed = serde_json::from_str(&text).unwrap_or_else(|error| {
                        panic!("the server sent what the protocol doesn't have ({error}): {text}")
                    });
                    {
                        let mut received = received.lock().unwrap();
                        apply_server_message(&mut received.state, &parsed);
                        received.raw.push(text);
                    }
                    changed.send_modify(|count| *count += 1);
                }
            })
        };
        Socket {
            writer,
            received,
            changed,
            reading,
        }
    }

    async fn send(&mut self, message: ClientMessage) {
        let text = serde_json::to_string(&message).unwrap();
        self.writer
            .send(tungstenite::Message::Text(text.into()))
            .await
            .unwrap();
    }

    fn state(&self) -> AppState {
        self.received
            .lock()
            .unwrap()
            .state
            .clone()
            .expect("no snapshot yet")
    }

    /// Waits until check is true of the state and the types of the messages so far; at most 10 seconds.
    async fn until(&self, check: impl Fn(&AppState, &[String]) -> bool) {
        let mut messages = self.changed.subscribe();
        let wait = async {
            loop {
                {
                    let received = self.received.lock().unwrap();
                    if let Some(state) = &received.state
                        && check(state, &received.types())
                    {
                        return;
                    }
                }
                messages.changed().await.expect("the socket is read");
            }
        };
        tokio::time::timeout(Duration::from_secs(10), wait)
            .await
            .expect("waited 10 seconds for the server");
    }

    async fn close(mut self) -> Received {
        let _ = self.writer.close().await;
        self.reading.abort();
        std::mem::take(&mut *self.received.lock().unwrap())
    }
}

#[tokio::test]
async fn records_a_session_over_ws() {
    if std::env::var("RECORD_IPAD_FIXTURES").as_deref() != Ok("1") {
        return;
    }
    let asked = Arc::new(AtomicBool::new(false));
    let renders = Arc::new(Renders::default());
    let store = memory_project_store(vec![
        LoadedProject {
            project: project("project-1", None, "Interview", "resolve-interview"),
            threads: vec![thread("thread-1", "Silences", "2026-11-17T09:00:00.000Z")],
        },
        LoadedProject {
            project: project("project-2", Some("/promo"), "Promo", "resolve-promo"),
            threads: vec![thread("thread-2", "Titles", "2026-11-16T09:00:00.000Z")],
        },
    ]);
    let turn_asked = asked.clone();
    let mut options = AppOptions::new(
        "0.0.0",
        Arc::new(store),
        Arc::new(move |_: SessionRequest| {
            Arc::new(Turn {
                asked: turn_asked.clone(),
            }) as Arc<dyn ProviderSession>
        }),
        Arc::new(|| Box::pin(async { open() })),
    );
    options.preview = Some(renders.clone());
    let app = create_app(options);
    let dist = tempfile::Builder::new()
        .prefix("linker-edit-ipad-fixtures-")
        .tempdir()
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let routes = router(HttpOptions::new(
        app,
        Web::Dir(dist.path().to_path_buf()),
        "secret",
    ));
    let server = tokio::spawn(async move { axum::serve(listener, routes).await });

    let mut socket = Socket::open(port).await;
    let emit = |event: PreviewEvent| {
        let render = renders.0.lock().unwrap()[0].clone();
        render(event);
    };

    socket.until(|_, _| true).await;
    socket.send(ClientMessage::RefreshResolve).await;
    socket
        .until(|s, _| matches!(s.meta.resolve, ResolveStatus::Connected { .. }))
        .await;
    socket
        .send(ClientMessage::SelectThread {
            thread_id: "thread-1".into(),
        })
        .await;
    socket
        .until(|s, _| s.meta.thread_id.as_deref() == Some("thread-1"))
        .await;
    socket
        .send(ClientMessage::SetApprovalMode {
            mode: ApprovalMode::Ask,
        })
        .await;
    socket
        .until(|s, _| s.meta.approval_mode == ApprovalMode::Ask)
        .await;
    socket
        .send(ClientMessage::Send {
            text: "Cut the silences".into(),
            compact_first: None,
        })
        .await;
    socket.until(|s, _| !s.meta.approvals.is_empty()).await;
    let id = socket.state().meta.approvals[0].id.clone();
    socket
        .send(ClientMessage::Approve { id, allow: true })
        .await;
    socket
        .until(|s, _| !s.meta.busy && s.items.iter().any(|item| item_id(item) == "a2"))
        .await;
    socket
        .send(ClientMessage::RenderPreview { timeline_id: None })
        .await;
    socket
        .until(|s, _| matches!(s.meta.preview, PreviewState::Rendering { .. }))
        .await;
    emit(PreviewEvent::Start {
        timeline: TimelineRef {
            id: "timeline-1".into(),
            name: "AI - Cut".into(),
        },
    });
    emit(PreviewEvent::Progress { percent: 40.0 });
    emit(PreviewEvent::Done {
        file: "/previews/preview-1.mp4".into(),
    });
    socket
        .until(|s, _| matches!(s.meta.preview, PreviewState::Ready { .. }))
        .await;
    // Promo isn't the project open in Resolve, so a message there goes back to the composer.
    socket
        .send(ClientMessage::SelectThread {
            thread_id: "thread-2".into(),
        })
        .await;
    socket
        .until(|s, _| s.meta.thread_id.as_deref() == Some("thread-2"))
        .await;
    socket
        .send(ClientMessage::Send {
            text: "Add titles".into(),
            compact_first: None,
        })
        .await;
    socket
        .until(|_, sent| sent.iter().any(|t| t == "refused") && sent.iter().any(|t| t == "toast"))
        .await;
    let received = socket.close().await;
    server.abort();
    assert!(asked.load(Ordering::SeqCst), "the turn asked for approval");

    let raw = &received.raw;
    let root = fixtures();
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("server")).unwrap();
    std::fs::create_dir_all(root.join("client")).unwrap();
    let mut ids = HashMap::new();
    for (index, message) in raw.iter().enumerate() {
        let value: Value = serde_json::from_str(&stable(message, &mut ids)).unwrap();
        let file = format!("{:03}-{}.json", index + 1, message_type(message));
        let json = serde_json::to_string_pretty(&value).unwrap();
        std::fs::write(root.join("server").join(file), format!("{json}\n")).unwrap();
    }
    for (name, message) in client_messages() {
        let json = serde_json::to_string_pretty(&message).unwrap();
        std::fs::write(
            root.join("client").join(format!("{name}.json")),
            format!("{json}\n"),
        )
        .unwrap();
    }
    let types = received.types();
    for expected in ["snapshot", "meta", "item", "refused", "toast"] {
        assert!(
            types.iter().any(|t| t == expected),
            "no {expected} in {types:?}"
        );
    }
}
