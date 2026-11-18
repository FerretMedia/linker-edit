//! Video and sound for the preview: ffmpeg decodes, GPUI draws the frames, cpal plays the sound
//! and keeps the clock. One engine ([`player::Player`]) for both the cut preview and a render;
//! [`sound::Sound`] for one sound file on its own (the library page's ▶).

pub mod audio;
pub mod ffmpeg;
pub mod player;
pub mod sound;
pub mod timeline;
pub mod video;
