//! What the window knows: the server's state as it arrives, nothing of its own.
//!
//! The window is one client of the server, like the browser. The server sends `ServerMessage`s
//! and takes `ClientMessage`s, the same types as on `/ws`, over a [`Link`]: in-process channels
//! for this computer's server, a `/ws` connection for another computer's. [`AppState`] is the
//! single entity that holds what arrived; views observe it and call [`AppState::send`].
//! A rule enforced only here isn't enforced: the server decides, the window shows.

use futures::{
    StreamExt,
    channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded},
};
use gpui::{App, AppContext, Context, Entity};

/// Stand-in until crates/protocol has the `/ws` types; then this becomes
/// `pub use linker_edit_protocol::ServerMessage;` and [`AppState::apply`] grows its match.
#[derive(Debug)]
pub enum ServerMessage {}

/// Stand-in, like [`ServerMessage`]: becomes `linker_edit_protocol::ClientMessage`.
#[derive(Debug)]
pub enum ClientMessage {}

/// What comes down a link: a message, or the link itself going up or down.
#[derive(Debug)]
pub enum LinkEvent {
    Connected,
    Disconnected,
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

pub struct AppState {
    /// Whether the link is up; the chat shows "Reconnecting…" while it isn't.
    pub connected: bool,
    /// Whether it was ever up, so the first wait reads "Connecting…", not "Lost connection".
    pub ever_connected: bool,
    commands: UnboundedSender<ClientMessage>,
}

impl AppState {
    /// Makes the state entity and pumps the link's events into it on the foreground thread.
    pub fn new(link: Link, cx: &mut App) -> Entity<AppState> {
        let Link {
            mut events,
            commands,
        } = link;
        let state = cx.new(|_| AppState {
            connected: false,
            ever_connected: false,
            commands,
        });
        let weak = state.downgrade();
        cx.spawn(async move |cx| {
            while let Some(event) = events.next().await {
                if weak.update(cx, |state, cx| state.apply(event, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        state
    }

    fn apply(&mut self, event: LinkEvent, cx: &mut Context<Self>) {
        match event {
            LinkEvent::Connected => {
                self.connected = true;
                self.ever_connected = true;
            }
            LinkEvent::Disconnected => self.connected = false,
            LinkEvent::Message(message) => match message {},
        }
        cx.notify();
    }

    /// Sends a command to the server. Nothing changes here until the server answers.
    pub fn send(&self, message: ClientMessage) {
        // A closed link drops it, as a closed socket would; the banner already says so.
        let _ = self.commands.unbounded_send(message);
    }
}
