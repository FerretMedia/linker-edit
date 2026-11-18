//! The library: folders of files the user reuses (music, sound effects, graphics, LUTs, footage). The port of
//! server/library. [`library::Library`] is the service the app holds; [`folders`] keeps the folders and what's
//! noted on files; [`serve`] says which file GET /library/<folder id>/<path> may send; [`tools`] are the agents'.

pub mod folders;
#[allow(clippy::module_inception)]
pub mod library;
pub mod scan;
pub mod search;
pub mod serve;
#[cfg(test)]
pub(crate) mod testing;
pub mod tools;

pub use library::{FileChange, Library, LibraryLink, LibraryOptions};
