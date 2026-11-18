//! The edit history: the Resolve project in Git, per turn. The port of server/history.

pub mod diff;
#[allow(clippy::module_inception)]
pub mod history;
pub mod model;
pub mod playable;
