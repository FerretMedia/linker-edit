//! Messages exchanged between the server and its clients (the GPUI window, the browser UI, the iPad).
//!
//! The port of apps/desktop/shared: `protocol.ts` (generated from this crate, see [`typescript`]),
//! `release-name.ts` and `project-icon.ts`.

mod number;
pub mod project_icon;
mod protocol;
pub mod release_name;
pub mod typescript;

pub use protocol::*;
