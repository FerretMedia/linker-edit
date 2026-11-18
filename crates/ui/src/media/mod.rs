//! Video and sound for the preview: ffmpeg decodes, GPUI draws the frames, cpal plays the sound
//! and keeps the clock. One engine ([`player::Player`]) for both the cut preview and a render.

pub mod audio;
pub mod ffmpeg;
pub mod player;
pub mod timeline;
pub mod video;
