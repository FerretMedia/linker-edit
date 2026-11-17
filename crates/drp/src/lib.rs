//! Reads and patches DaVinci Resolve .drp project and .drt timeline files without Resolve.
//! The port of `packages/drp`; its README has the format map.

pub mod blobs;
mod patch;
mod read;
mod version;
mod xml;
pub mod zip;

pub use patch::{Edit, patch_drp};
pub use read::{
    DrpFile, MediaFolder, MediaItem, Project, Timeline, TimelineClip, Track, TrackType,
    media_items, read_drp,
};
pub use version::{VersionCheck, check_version};

/// Why a file couldn't be read or patched. The message is written for the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct Error(pub String);

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(Error(message.into()))
}
