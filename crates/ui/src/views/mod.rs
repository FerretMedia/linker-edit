//! The views the workspace lays out. One file per region of +page.svelte; a view with state of
//! its own (scroll, a draft, a search query) is an entity, one without is a `RenderOnce`.

mod chat;
mod panel;
mod sidebar;

pub use chat::Chat;
pub use panel::{Panel, SidePanel};
pub use sidebar::Sidebar;
