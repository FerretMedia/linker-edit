//! The views the workspace lays out. One file per region of +page.svelte; a view with state of
//! its own (scroll, a draft, a search query) is an entity, one without is a `RenderOnce`.

pub mod approval_card;
mod chat;
pub mod composer;
mod panel;
pub mod player_controls;
pub mod preview_panel;
mod sidebar;
pub mod thread_starter;
pub mod timeline_player;
pub mod tool_row;

pub use chat::{CHAT_MAX_WIDTH, Chat, ChatEvent, Row, rows};
pub use panel::{Panel, SidePanel};
pub use preview_panel::{PreviewEvent, PreviewPanel};
pub use sidebar::Sidebar;
