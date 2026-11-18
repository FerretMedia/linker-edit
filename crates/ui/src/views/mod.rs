//! The views the workspace lays out. One file per region of +page.svelte; a view with state of
//! its own (scroll, a draft, a search query) is an entity, one without is a `RenderOnce`.

pub mod add_project;
pub mod approval_card;
mod chat;
pub mod collection_editor;
pub mod collection_icon;
pub mod composer;
pub mod computer_picker;
pub mod context_menu;
pub mod dialog;
pub mod folder_picker;
mod panel;
pub mod project_icon;
pub mod project_settings;
mod sidebar;
mod sidebar_footer;
mod sidebar_rows;
#[cfg(test)]
mod sidebar_tests;
pub mod thread_starter;
pub mod tool_row;

pub use chat::{CHAT_MAX_WIDTH, Chat, ChatEvent, Row, rows};
pub use panel::{Panel, SidePanel};
pub use sidebar::{RowAction, SIDEBAR_WIDTH, Sidebar, SidebarAction, SidebarEvent, Variant};
pub use sidebar_footer::{StatusLine, resolve_status_line};
