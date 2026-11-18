//! The views the workspace lays out. One file per region of +page.svelte; a view with state of
//! its own (scroll, a draft, a search query) is an entity, one without is a `RenderOnce`.

pub mod app_update_card;
pub mod approval_card;
mod chat;
pub mod choice_select;
pub mod clip_strip;
pub mod collections_settings;
pub mod composer;
pub mod history_panel;
pub mod library_folder_dialog;
pub mod library_page;
pub mod linker_connect;
mod panel;
pub mod project_file_panel;
pub mod remote_access;
pub mod settings_page;
pub mod settings_parts;
mod sidebar;
pub mod thread_starter;
pub mod tool_row;

pub use chat::{CHAT_MAX_WIDTH, Chat, ChatEvent, Row, rows};
pub use panel::{Panel, SidePanel};
pub use sidebar::{Sidebar, SidebarEvent};
