// What the media modules' tests share: temporary folders, files, links, a probe that counts, and ffmpeg.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use linker_edit_protocol::LibraryFolder;

use super::scan::{Probe, ProbeFile};
use crate::library::folders::library_kind;
use crate::transcribe::tools::find_on_path;

pub fn temp() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("linker-edit-media-")
        .tempdir()
        .unwrap()
}

pub fn write(file: &Path, contents: &str) {
    write_bytes(file, contents.as_bytes());
}

pub fn write_bytes(file: &Path, contents: &[u8]) {
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, contents).unwrap();
}

pub fn folder(path: &str, kind: &str, id: &str) -> LibraryFolder {
    LibraryFolder {
        id: id.into(),
        kind: library_kind(&kind.into()).unwrap(),
        label: "Music".into(),
        path: path.into(),
        note: String::new(),
    }
}

/// Makes a link, or says it can't: Windows needs developer mode for links to files. Folders get a junction there,
/// which needs nothing.
pub fn link(target: &Path, at: &Path, folder: bool) -> bool {
    #[cfg(windows)]
    {
        if folder {
            return std::process::Command::new("cmd")
                .arg("/c")
                .arg("mklink")
                .arg("/J")
                .arg(at)
                .arg(target)
                .output()
                .is_ok_and(|output| output.status.success());
        }
        std::os::windows::fs::symlink_file(target, at).is_ok()
    }
    #[cfg(not(windows))]
    {
        let _ = folder;
        std::os::unix::fs::symlink(target, at).is_ok()
    }
}

/// A probe that says every file is 12.5 seconds of stereo at -14 LUFS, and names the files it was asked about.
#[derive(Default)]
pub struct CountingProbe {
    calls: Mutex<Vec<String>>,
}

impl CountingProbe {
    pub fn calls_sorted(&self) -> Vec<String> {
        let mut calls = self.calls.lock().unwrap().clone();
        calls.sort();
        calls
    }

    pub fn clear(&self) {
        self.calls.lock().unwrap().clear();
    }
}

#[async_trait]
impl ProbeFile for CountingProbe {
    async fn probe(&self, file: &Path, loudness: bool) -> anyhow::Result<Probe> {
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        self.calls.lock().unwrap().push(name);
        Ok(Probe {
            duration: Some(12.5),
            channels: Some(2),
            loudness: loudness.then_some(-14.0),
        })
    }
}

pub fn counting_probe() -> Arc<CountingProbe> {
    Arc::default()
}

/// A short 16-bit PCM WAV: a quiet sine, so ffmpeg has something to measure.
pub fn wav(seconds: f64, channels: u16, rate: u32) -> Vec<u8> {
    let samples = (seconds * f64::from(rate)).round() as usize;
    let mut data = Vec::with_capacity(samples * usize::from(channels) * 2);
    for i in 0..samples {
        let value =
            ((2.0 * std::f64::consts::PI * 440.0 * i as f64) / f64::from(rate)).sin() * 8000.0;
        for _ in 0..channels {
            data.extend_from_slice(&(value.round() as i16).to_le_bytes());
        }
    }
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

/// ffmpeg for tests: one on PATH, or the one `npm ci` put in the repo's node_modules (ffmpeg-static). None, and a
/// note on stderr, when neither is there.
pub fn test_ffmpeg() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let npm = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../node_modules/ffmpeg-static")
        .join(name);
    let found = find_on_path("ffmpeg").or_else(|| npm.is_file().then_some(npm));
    if found.is_none() {
        eprintln!("Skipped: no ffmpeg on PATH or in node_modules/ffmpeg-static (run npm ci).");
    }
    found
}
