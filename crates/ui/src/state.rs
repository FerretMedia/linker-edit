//! What the window knows: the server's state as it arrives, nothing of its own.
//!
//! The window is one client of the server, like the browser. The server sends `ServerMessage`s
//! and takes `ClientMessage`s, the same types as on `/ws` (crates/protocol), over a [`Link`]:
//! in-process channels for this computer's server ([`Link::pair`]), a `/ws` connection for
//! another computer's ([`crate::transport::connect`]). [`AppState`] is the single entity that
//! holds what arrived, as src/lib/connection.svelte.ts and appState.ts do; views observe it and
//! call [`AppState::send`]. What +page.svelte derives from it (the open project and thread,
//! whether a message can go) are methods here, so every view derives it the same way.
//! A rule enforced only here isn't enforced: the server decides, the window shows.

use std::collections::HashMap;
use std::time::Duration;

use futures::{
    FutureExt, StreamExt,
    channel::{
        mpsc::{UnboundedReceiver, UnboundedSender, unbounded},
        oneshot,
    },
    future::BoxFuture,
};
use gpui::{App, AppContext, Context, Entity, EventEmitter, Task};
use linker_edit_protocol::{
    self as protocol, ChatItem, CollectionSummary, FolderListing, HistoryEntry, LibraryView,
    OrError, PreviewState, ProjectSummary, ResolveLibrary, ThreadSummary, Tone,
};

pub use linker_edit_protocol::{ClientMessage, ServerMessage};

/// What comes down a link: a message, or the link itself going up or down.
#[derive(Debug)]
pub enum LinkEvent {
    Connected,
    Disconnected,
    /// The server turned the link's session away (a Linker Connect Bearer session that ended):
    /// it won't be retried; sign in to that computer again.
    SignedOut,
    Message(ServerMessage),
}

/// The window's end of a link to a server.
pub struct Link {
    pub events: UnboundedReceiver<LinkEvent>,
    pub commands: UnboundedSender<ClientMessage>,
}

/// The server's end of the same link.
pub struct ServerEnd {
    pub events: UnboundedSender<LinkEvent>,
    pub commands: UnboundedReceiver<ClientMessage>,
}

impl Link {
    /// Two connected ends: the window takes the `Link`, the server (or a test) the `ServerEnd`.
    pub fn pair() -> (Link, ServerEnd) {
        let (events_tx, events_rx) = unbounded();
        let (commands_tx, commands_rx) = unbounded();
        (
            Link {
                events: events_rx,
                commands: commands_tx,
            },
            ServerEnd {
                events: events_tx,
                commands: commands_rx,
            },
        )
    }
}

/// A short message in the corner, gone after a few seconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub id: u64,
    pub text: String,
    pub tone: Tone,
}

const TOAST_TIME: Duration = Duration::from_secs(6);

/// A project's edit history, as the history panel asked for it.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectHistory {
    pub project_id: String,
    pub entries: Vec<HistoryEntry>,
}

/// A project's PROJECT.md.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectFile {
    pub project_id: String,
    pub file: String,
    pub text: String,
}

/// What views hear besides a change: things that happen once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppEvent {
    /// The message wasn't sent; it goes back in that thread's composer.
    Refused { thread_id: String, text: String },
}

pub struct AppState {
    /// Whether the link is up; the chat shows "Reconnecting…" while it isn't.
    pub connected: bool,
    /// Whether it was ever up, so the first wait reads "Connecting…", not "Lost connection".
    pub ever_connected: bool,
    /// The server ended this link's session; see [`LinkEvent::SignedOut`].
    pub signed_out: bool,
    /// The server's state: the snapshot with the patches since. None until the first snapshot.
    pub app: Option<protocol::AppState>,
    /// None until the history panel first asks for it.
    pub history: Option<ProjectHistory>,
    /// None until the project file panel first asks for it.
    pub project_file: Option<ProjectFile>,
    /// None until the add-project dialog asks for it (see [`AppState::load_library`]).
    pub library: Option<OrError<ResolveLibrary>>,
    /// The library page's library, while the page is open.
    pub library_view: Option<LibraryView>,
    pub toasts: Vec<Toast>,
    next_toast: u64,
    folder_request: u32,
    folder_replies: HashMap<u32, oneshot::Sender<OrError<FolderListing>>>,
    commands: UnboundedSender<ClientMessage>,
    /// This computer's server, by its http(s) address; None when the window doesn't know it.
    pub home: Option<String>,
    /// The server the link goes to now: `home`, or another computer's (see [`AppState::relink`]).
    pub server: Option<String>,
    /// Folds the link's events into the state; replaced, and so stopped, by a new link.
    pump: Option<Task<()>>,
}

impl EventEmitter<AppEvent> for AppState {}

impl AppState {
    /// Makes the state entity and pumps the link's events into it on the foreground thread.
    pub fn new(link: Link, cx: &mut App) -> Entity<AppState> {
        let Link { events, commands } = link;
        let state = cx.new(|_| AppState {
            connected: false,
            ever_connected: false,
            signed_out: false,
            app: None,
            history: None,
            project_file: None,
            library: None,
            library_view: None,
            toasts: Vec::new(),
            next_toast: 0,
            folder_request: 0,
            folder_replies: HashMap::new(),
            commands,
            home: None,
            server: None,
            pump: None,
        });
        state.update(cx, |state, cx| state.pump = Some(Self::pump(events, cx)));
        state
    }

    fn pump(mut events: UnboundedReceiver<LinkEvent>, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                if this.update(cx, |state, cx| state.apply(event, cx)).is_err() {
                    break;
                }
            }
        })
    }

    /// Puts the window on another link, such as another computer's server (`server` is its
    /// address): what came over the old one is forgotten, as when the browser loads another page.
    pub fn relink(&mut self, server: String, link: Link, cx: &mut Context<Self>) {
        let Link { events, commands } = link;
        for (_, reply) in self.folder_replies.drain() {
            let _ = reply.send(OrError::Error {
                error: "Lost the connection to the Linker-Edit server.".into(),
            });
        }
        self.commands = commands;
        self.pump = Some(Self::pump(events, cx));
        self.server = Some(server);
        self.connected = false;
        self.ever_connected = false;
        self.signed_out = false;
        self.app = None;
        self.history = None;
        self.project_file = None;
        self.library = None;
        self.library_view = None;
        cx.notify();
    }

    /// The window shows another computer's Linker-Edit, not this computer's.
    pub fn away(&self) -> bool {
        match (&self.home, &self.server) {
            (Some(home), Some(server)) => origin(home) != origin(server),
            _ => false,
        }
    }

    /// Folds one event into the state, as connection.svelte.ts's onmessage and appState.ts's
    /// applyServerMessage do.
    pub fn apply(&mut self, event: LinkEvent, cx: &mut Context<Self>) {
        match event {
            LinkEvent::Connected => {
                self.connected = true;
                self.ever_connected = true;
                self.signed_out = false;
            }
            LinkEvent::Disconnected | LinkEvent::SignedOut => {
                self.signed_out = matches!(event, LinkEvent::SignedOut);
                self.connected = false;
                for (_, reply) in self.folder_replies.drain() {
                    let _ = reply.send(OrError::Error {
                        error: "Lost the connection to the Linker-Edit server.".into(),
                    });
                }
            }
            LinkEvent::Message(message) => self.apply_message(message, cx),
        }
        cx.notify();
    }

    fn apply_message(&mut self, message: ServerMessage, cx: &mut Context<Self>) {
        match message {
            // The browser reloads when a snapshot comes from a newer server, to get its UI. This
            // window's UI is compiled in; a mismatch with another computer is the computer
            // picker's to show.
            ServerMessage::Snapshot { state } => self.app = Some(*state),
            ServerMessage::Meta { meta } => {
                if let Some(app) = &mut self.app {
                    app.meta = *meta;
                }
            }
            ServerMessage::Item { thread_id, item } => {
                // An item of a thread the client just left can still be on its way.
                let Some(app) = &mut self.app else { return };
                if app.meta.thread_id.as_deref() != Some(thread_id.as_str()) {
                    return;
                }
                match app
                    .items
                    .iter_mut()
                    .find(|known| item_id(known) == item_id(&item))
                {
                    Some(known) => *known = item,
                    None => app.items.push(item),
                }
            }
            ServerMessage::History {
                project_id,
                entries,
            } => {
                self.history = Some(ProjectHistory {
                    project_id,
                    entries,
                })
            }
            ServerMessage::ProjectFile {
                project_id,
                file,
                text,
            } => {
                self.project_file = Some(ProjectFile {
                    project_id,
                    file,
                    text,
                })
            }
            ServerMessage::Library { library } => self.library = Some(library),
            ServerMessage::LibraryView(protocol::LibraryServerMessage::LibraryView { view }) => {
                self.library_view = Some(view)
            }
            ServerMessage::Folders {
                request_id,
                listing,
            } => {
                if let Some(reply) = self.folder_replies.remove(&request_id) {
                    let _ = reply.send(listing);
                }
            }
            ServerMessage::Refused { thread_id, text } => {
                cx.emit(AppEvent::Refused { thread_id, text })
            }
            ServerMessage::Toast { text, tone } => self.toast(text, tone, cx),
        }
    }

    /// Sends a command to the server. Nothing changes here until the server answers.
    pub fn send(&self, message: ClientMessage) {
        // A closed link drops it, as a closed socket would; the banner already says so.
        let _ = self.commands.unbounded_send(message);
    }

    /// The folders in a folder on the server's computer; an empty path is its home folder.
    pub fn browse_folders(&mut self, path: String) -> BoxFuture<'static, OrError<FolderListing>> {
        if !self.connected {
            return futures::future::ready(OrError::Error {
                error: "Not connected to the Linker-Edit server.".into(),
            })
            .boxed();
        }
        self.folder_request += 1;
        let request_id = self.folder_request;
        let (reply, answer) = oneshot::channel();
        self.folder_replies.insert(request_id, reply);
        self.send(ClientMessage::BrowseFolders { request_id, path });
        answer
            .map(|answer| {
                answer.unwrap_or(OrError::Error {
                    error: "Lost the connection to the Linker-Edit server.".into(),
                })
            })
            .boxed()
    }

    /// Asks for the Resolve library again; None until it arrives.
    pub fn load_library(&mut self, cx: &mut Context<Self>) {
        self.library = None;
        self.send(ClientMessage::LoadLibrary);
        cx.notify();
    }

    /// Shows a toast; the same text again replaces the older one.
    pub fn toast(&mut self, text: String, tone: Tone, cx: &mut Context<Self>) {
        self.next_toast += 1;
        let id = self.next_toast;
        self.toasts.retain(|toast| toast.text != text);
        self.toasts.push(Toast { id, text, tone });
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_TIME).await;
            let _ = this.update(cx, |this, cx| this.dismiss(id, cx));
        })
        .detach();
        cx.notify();
    }

    pub fn dismiss(&mut self, id: u64, cx: &mut Context<Self>) {
        self.toasts.retain(|toast| toast.id != id);
        cx.notify();
    }

    // --- What +page.svelte derives ---

    /// The project of the thread in view.
    pub fn project(&self) -> Option<&ProjectSummary> {
        let app = self.app.as_ref()?;
        let id = app.meta.project_id.as_deref()?;
        app.meta.projects.iter().find(|project| project.id == id)
    }

    /// The thread in view.
    pub fn thread(&self) -> Option<&ThreadSummary> {
        let id = self.app.as_ref()?.meta.thread_id.as_deref()?;
        self.project()?
            .threads
            .iter()
            .find(|thread| thread.id == id)
    }

    /// The chat header's title.
    pub fn chat_title(&self) -> &str {
        self.thread().map_or("New thread", |thread| &thread.title)
    }

    /// Resolve is rendering a preview of this project.
    pub fn rendering(&self) -> bool {
        self.app
            .as_ref()
            .is_some_and(|app| matches!(app.meta.preview, PreviewState::Rendering { .. }))
    }

    /// The workspace or series the open project is in, or sets up.
    pub fn collection(&self) -> Option<&CollectionSummary> {
        let project = self.project()?;
        self.app
            .as_ref()?
            .meta
            .collections
            .iter()
            .find(|collection| {
                collection.project_ids.contains(&project.id)
                    || collection.setup_project_id.as_deref() == Some(project.id.as_str())
            })
    }

    /// Why a thread starter can't start now; None when it can.
    pub fn start_blocked(&self) -> Option<String> {
        if !self.connected {
            return Some("Not connected.".into());
        }
        let app = self.app.as_ref()?;
        if app.meta.busy {
            return Some("Wait for the agent to finish.".into());
        }
        if let Some(blocked) = &app.meta.blocked {
            return Some(blocked.message.clone());
        }
        self.rendering().then(|| "Wait for the render.".into())
    }

    /// Whether `draft` can be sent now.
    pub fn can_send(&self, draft: &str) -> bool {
        let Some(app) = &self.app else { return false };
        self.connected
            && app.meta.thread_id.is_some()
            && !app.meta.busy
            && app.meta.blocked.is_none()
            && !self.rendering()
            && !draft.trim().is_empty()
    }

    /// Something is using Resolve: a turn in any thread, opening a project, or applying a timeline.
    pub fn resolve_busy(&self) -> bool {
        self.app.as_ref().is_some_and(|app| {
            app.meta.busy
                || app.meta.opening.is_some()
                || app.meta.applying.is_some()
                || app
                    .meta
                    .projects
                    .iter()
                    .any(|project| project.threads.iter().any(|thread| thread.busy))
        })
    }

    /// The history of the project in view, once it arrived.
    pub fn history_entries(&self) -> Option<&[HistoryEntry]> {
        let history = self.history.as_ref()?;
        (Some(history.project_id.as_str()) == self.project_id()).then_some(&history.entries[..])
    }

    /// The PROJECT.md of the project in view, once it arrived.
    pub fn project_file_in_view(&self) -> Option<&ProjectFile> {
        let file = self.project_file.as_ref()?;
        (Some(file.project_id.as_str()) == self.project_id()).then_some(file)
    }

    pub fn project_id(&self) -> Option<&str> {
        self.app.as_ref()?.meta.project_id.as_deref()
    }

    pub fn thread_id(&self) -> Option<&str> {
        self.app.as_ref()?.meta.thread_id.as_deref()
    }
}

/// `new URL(url).origin`, near enough: the scheme, host and port, without a path or a slash.
pub fn origin(url: &str) -> String {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://").unwrap_or(("http", url));
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    format!("{}://{}", scheme.to_lowercase(), host.to_lowercase())
}

/// Every chat item's id.
pub fn item_id(item: &ChatItem) -> &str {
    match item {
        ChatItem::User { id, .. }
        | ChatItem::Assistant { id, .. }
        | ChatItem::Tool { id, .. }
        | ChatItem::Notice { id, .. } => id,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use gpui::TestAppContext;
    use linker_edit_protocol::{AppMeta, Blocked, ToolStatus};

    /// A server state as the TS server sends it with one project and an empty thread
    /// (crates/protocol's fixture), for tests to change.
    pub fn snapshot() -> protocol::AppState {
        let mut state: protocol::AppState = serde_json::from_str(include_str!(
            "../../protocol/tests/fixtures/ipad/server/001-snapshot.json"
        ))
        .map(|message: ServerMessage| match message {
            ServerMessage::Snapshot { state } => *state,
            _ => unreachable!(),
        })
        .expect("the fixture is a snapshot");
        state.items.clear();
        state
    }

    fn meta(state: &protocol::AppState) -> AppMeta {
        state.meta.clone()
    }

    fn open(cx: &mut TestAppContext) -> (Entity<AppState>, ServerEnd) {
        let (link, server) = Link::pair();
        let state = cx.update(|cx| AppState::new(link, cx));
        (state, server)
    }

    #[gpui::test]
    fn applies_a_snapshot_then_patches(cx: &mut TestAppContext) {
        let (state, server) = open(cx);
        let snapshot = snapshot();
        let thread = snapshot.meta.thread_id.clone().unwrap();
        let send = |event| server.events.unbounded_send(event).unwrap();
        send(LinkEvent::Connected);
        // Patches before a snapshot have nothing to apply to.
        send(LinkEvent::Message(ServerMessage::Meta {
            meta: Box::new(meta(&snapshot)),
        }));
        cx.run_until_parked();
        assert!(state.read_with(cx, |state, _| state.app.is_none()));

        send(LinkEvent::Message(ServerMessage::Snapshot {
            state: Box::new(snapshot.clone()),
        }));
        let item = |id: &str, text: &str| ChatItem::Assistant {
            id: id.into(),
            text: text.into(),
        };
        send(LinkEvent::Message(ServerMessage::Item {
            thread_id: thread.clone(),
            item: item("a", "Hel"),
        }));
        // A streamed item replaces itself.
        send(LinkEvent::Message(ServerMessage::Item {
            thread_id: thread.clone(),
            item: item("a", "Hello"),
        }));
        // An item of another thread is still on its way after a switch.
        send(LinkEvent::Message(ServerMessage::Item {
            thread_id: "elsewhere".into(),
            item: item("b", "late"),
        }));
        let mut busy = meta(&snapshot);
        busy.busy = true;
        busy.blocked = Some(Blocked {
            message: "Wait".into(),
            can_open: false,
        });
        send(LinkEvent::Message(ServerMessage::Meta {
            meta: Box::new(busy),
        }));
        cx.run_until_parked();
        state.read_with(cx, |state, _| {
            let app = state.app.as_ref().unwrap();
            assert_eq!(app.items, [item("a", "Hello")]);
            assert!(app.meta.busy);
            assert!(!state.can_send("hi"));
            assert_eq!(
                state.start_blocked().as_deref(),
                Some("Wait for the agent to finish.")
            );
            assert_eq!(state.project().map(|p| p.name.as_str()), Some("Interview"));
            assert!(state.resolve_busy());
        });

        // The next snapshot replaces all of it, as after a reconnect.
        send(LinkEvent::Disconnected);
        send(LinkEvent::Connected);
        send(LinkEvent::Message(ServerMessage::Snapshot {
            state: Box::new(snapshot.clone()),
        }));
        cx.run_until_parked();
        state.read_with(cx, |state, _| {
            assert!(state.app.as_ref().unwrap().items.is_empty());
            assert!(state.can_send("hi"));
            assert!(!state.can_send("  \n"));
        });
    }

    #[gpui::test]
    fn answers_a_folder_request_and_gives_up_on_it_when_the_link_drops(cx: &mut TestAppContext) {
        let (state, mut server) = open(cx);
        server.events.unbounded_send(LinkEvent::Connected).unwrap();
        cx.run_until_parked();
        let first = state.update(cx, |state, _| state.browse_folders("C:\\".into()));
        let second = state.update(cx, |state, _| state.browse_folders("D:\\".into()));
        let asked: Vec<ClientMessage> =
            std::iter::from_fn(|| server.commands.try_recv().ok()).collect();
        assert_eq!(
            asked,
            [
                ClientMessage::BrowseFolders {
                    request_id: 1,
                    path: "C:\\".into()
                },
                ClientMessage::BrowseFolders {
                    request_id: 2,
                    path: "D:\\".into()
                },
            ]
        );
        let listing = FolderListing {
            dir: "C:\\".into(),
            is_project: false,
            entries: vec![],
            roots: vec!["C:\\".into()],
            home: "C:\\Users\\you\\".into(),
        };
        server
            .events
            .unbounded_send(LinkEvent::Message(ServerMessage::Folders {
                request_id: 1,
                listing: OrError::Ok(listing.clone()),
            }))
            .unwrap();
        server
            .events
            .unbounded_send(LinkEvent::Disconnected)
            .unwrap();
        cx.run_until_parked();
        assert_eq!(first.now_or_never(), Some(OrError::Ok(listing)));
        assert!(matches!(
            second.now_or_never(),
            Some(OrError::Error { error }) if error.starts_with("Lost the connection")
        ));
        let offline = state.update(cx, |state, _| state.browse_folders(String::new()));
        assert!(matches!(
            offline.now_or_never(),
            Some(OrError::Error { .. })
        ));
    }

    #[gpui::test]
    fn passes_on_a_refused_message_and_shows_toasts(cx: &mut TestAppContext) {
        let (state, server) = open(cx);
        let refused = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = refused.clone();
        cx.update(|cx| {
            cx.subscribe(&state, move |_, event: &AppEvent, _| {
                seen.borrow_mut().push(event.clone())
            })
            .detach()
        });
        for message in [
            ServerMessage::Refused {
                thread_id: "t".into(),
                text: "cut it".into(),
            },
            ServerMessage::Toast {
                text: "Saved".into(),
                tone: Tone::Info,
            },
            ServerMessage::Toast {
                text: "Saved".into(),
                tone: Tone::Info,
            },
        ] {
            server
                .events
                .unbounded_send(LinkEvent::Message(message))
                .unwrap();
        }
        cx.run_until_parked();
        assert_eq!(
            *refused.borrow(),
            [AppEvent::Refused {
                thread_id: "t".into(),
                text: "cut it".into()
            }]
        );
        // The same text twice shows once; it goes after a while.
        assert_eq!(state.read_with(cx, |state, _| state.toasts.len()), 1);
        cx.executor().advance_clock(TOAST_TIME);
        cx.run_until_parked();
        assert!(state.read_with(cx, |state, _| state.toasts.is_empty()));
    }

    #[test]
    fn every_item_has_its_id() {
        let tool = ChatItem::Tool {
            id: "t1".into(),
            name: "Bash".into(),
            input: serde_json::json!({}),
            status: ToolStatus::Running,
            output: None,
        };
        assert_eq!(item_id(&tool), "t1");
    }
}
