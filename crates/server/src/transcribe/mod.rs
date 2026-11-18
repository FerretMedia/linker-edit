//! Speech to text on this machine with whisper.cpp. The port of server/transcribe. [`transcriber::Transcriber`]
//! is the service the app holds; [`tools`] finds ffmpeg and downloads the engine; [`whisper`] runs it.

pub mod tools;
pub mod transcriber;
pub mod whisper;

pub use transcriber::{
    DownloadTools, Transcriber, TranscriberOptions, TranscriptResult, describe_results,
};
