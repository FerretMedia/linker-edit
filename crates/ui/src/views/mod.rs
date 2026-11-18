//! The views the workspace lays out. One file per region of +page.svelte; a view with state of
//! its own (scroll, a draft, a search query) is an entity, one without is a `RenderOnce`.

pub mod add_project;
pub mod app_update_card;
pub mod approval_card;
mod chat;
pub mod choice_select;
pub mod clip_strip;
pub mod collections_settings;
pub mod composer;
pub mod computer_picker;
pub mod context_menu;
pub mod dialog;
pub mod folder_picker;
pub mod history_panel;
pub mod library_folder_dialog;
pub mod library_page;
pub mod linker_connect;
mod panel;
pub mod player_controls;
pub mod preview_panel;
pub mod project_file_panel;
pub mod project_icon;
pub mod project_settings;
pub mod remote_access;
pub mod settings_page;
pub mod settings_parts;
mod sidebar;
mod sidebar_footer;
mod sidebar_rows;
#[cfg(test)]
mod sidebar_tests;
pub mod thread_starter;
pub mod timeline_player;
pub mod tool_row;

pub use chat::{CHAT_MAX_WIDTH, Chat, ChatEvent, Row, rows};
pub use panel::{Panel, SidePanel};
pub use preview_panel::{PreviewEvent, PreviewPanel};
pub use sidebar::{RowAction, SIDEBAR_WIDTH, Sidebar, SidebarAction, SidebarEvent, Variant};
