// Reads what's in a library folder: each file's name, size and date, and with the ffmpeg Linker-Edit ships its
// duration, channels and loudness. Kept per folder in ~/LinkerEdit/cache/library/<folder id>.json; a rescan only
// measures files that are new or changed since.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use anyhow::bail;
use async_trait::async_trait;
use linker_edit_protocol::{LibraryFolder, LibraryKind};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::js;
use crate::transcribe::tools::run_process;

pub const AUDIO: &[&str] = &[
    ".wav", ".mp3", ".aif", ".aiff", ".flac", ".m4a", ".aac", ".ogg", ".opus", ".wma", ".caf",
];
pub const VIDEO: &[&str] = &[
    ".mov", ".mp4", ".m4v", ".mxf", ".avi", ".mkv", ".webm", ".mts", ".m2ts", ".braw", ".r3d",
    ".mpg", ".mpeg",
];
pub const IMAGE: &[&str] = &[
    ".png", ".jpg", ".jpeg", ".tif", ".tiff", ".psd", ".exr", ".dpx", ".tga", ".bmp", ".gif",
    ".webp", ".heic", ".svg",
];
pub const LUT: &[&str] = &[
    ".cube", ".3dl", ".dat", ".mga", ".m3d", ".look", ".ilut", ".olut", ".csp",
];

/// Which files each kind of folder lists; "other" lists every file.
fn kind_files(kind: LibraryKind) -> Option<&'static [&'static [&'static str]]> {
    match kind {
        LibraryKind::Music | LibraryKind::Sfx => Some(&[AUDIO, VIDEO]),
        LibraryKind::Graphics => Some(&[IMAGE, VIDEO]),
        LibraryKind::Luts => Some(&[LUT]),
        LibraryKind::Footage => Some(&[VIDEO, IMAGE, AUDIO]),
        LibraryKind::Other => None,
    }
}

const MAX_FILES: usize = 20_000;
const MAX_DEPTH: usize = 12;
const PARALLEL: usize = 3;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedFile {
    /// Inside the folder, with / between folders.
    pub rel: String,
    pub size: u64,
    #[serde(serialize_with = "js::num")]
    pub mtime_ms: f64,
    #[serde(serialize_with = "js::opt_num")]
    pub duration: Option<f64>,
    pub channels: Option<u32>,
    #[serde(serialize_with = "js::opt_num")]
    pub loudness: Option<f64>,
    /// False when ffmpeg wasn't there to measure it, so the next scan tries again.
    pub measured: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderCache {
    pub version: u32,
    /// The folder this was read from; a folder moved elsewhere starts over.
    pub path: String,
    pub scanned_at: String,
    pub files: Vec<ScannedFile>,
    /// More files than a library folder lists.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Probe {
    pub duration: Option<f64>,
    pub channels: Option<u32>,
    pub loudness: Option<f64>,
}

/// Measures one file; loudness only when asked, because it decodes all of the audio.
#[async_trait]
pub trait ProbeFile: Send + Sync {
    async fn probe(&self, file: &Path, loudness: bool) -> anyhow::Result<Probe>;
}

pub fn cache_file(cache_dir: &Path, folder_id: &str) -> PathBuf {
    static UNSAFE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^A-Za-z0-9_-]").unwrap());
    cache_dir.join(format!("{}.json", UNSAFE.replace_all(folder_id, "_")))
}

pub fn read_cache(cache_dir: &Path, folder: &LibraryFolder) -> Option<FolderCache> {
    let text = std::fs::read_to_string(cache_file(cache_dir, &folder.id)).ok()?;
    let cache: FolderCache = serde_json::from_str(&text).ok()?;
    (cache.version == VERSION && cache.path == folder.path).then_some(cache)
}

fn write_cache(cache_dir: &Path, folder_id: &str, cache: &FolderCache) -> anyhow::Result<()> {
    std::fs::create_dir_all(cache_dir)?;
    let file = cache_file(cache_dir, folder_id);
    let temp = PathBuf::from(format!("{}.tmp", file.display()));
    std::fs::write(&temp, serde_json::to_string(cache)?)?;
    std::fs::rename(&temp, &file)?;
    Ok(())
}

pub fn remove_cache(cache_dir: &Path, folder_id: &str) -> std::io::Result<()> {
    match std::fs::remove_file(cache_file(cache_dir, folder_id)) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn has_ext(set: &[&str], name: &str) -> bool {
    set.contains(&js::extname(name).to_lowercase().as_str())
}

pub fn lists_file(kind: LibraryKind, name: &str) -> bool {
    kind_files(kind).is_none_or(|sets| sets.iter().any(|set| has_ext(set, name)))
}

/// The files in a folder and the folders inside it, as paths with /, and whether there were more than a folder
/// lists. Hidden files and links are skipped: a link could lead out of the folder, and the library only reaches
/// what's inside it.
pub fn list_folder(root: &Path, kind: LibraryKind) -> std::io::Result<(Vec<String>, bool)> {
    fn walk(
        dir: &Path,
        rel: &str,
        depth: usize,
        kind: LibraryKind,
        files: &mut Vec<String>,
        truncated: &mut bool,
    ) -> std::io::Result<()> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if depth == 0 => return Err(error),
            Err(_) => return Ok(()),
        };
        let mut entries: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|entry| (entry.file_name().to_string_lossy().into_owned(), entry))
            .collect();
        entries.sort_by(|a, b| js::locale_compare(&a.0, &b.0));
        for (name, entry) in entries {
            if files.len() >= MAX_FILES {
                *truncated = true;
                return Ok(());
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if name.starts_with('.') || file_type.is_symlink() {
                continue;
            }
            let child_rel = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            if file_type.is_dir() {
                if depth < MAX_DEPTH {
                    walk(
                        &dir.join(&name),
                        &child_rel,
                        depth + 1,
                        kind,
                        files,
                        truncated,
                    )?;
                }
            } else if file_type.is_file() && lists_file(kind, &name) {
                files.push(child_rel);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    let mut truncated = false;
    walk(root, "", 0, kind, &mut files, &mut truncated)?;
    Ok((files, truncated))
}

/// Which files get measured, and whether for loudness: music and sound effects are, footage only for length.
fn measuring(kind: LibraryKind, rel: &str) -> (bool, bool) {
    let audio = has_ext(AUDIO, rel);
    let video = has_ext(VIDEO, rel);
    if !audio && !video {
        return (false, false);
    }
    (
        true,
        audio || kind == LibraryKind::Music || kind == LibraryKind::Sfx,
    )
}

pub type OnScanProgress = Arc<dyn Fn(usize, usize) + Send + Sync>;

pub struct ScanOptions {
    pub cache_dir: PathBuf,
    pub probe: Arc<dyn ProbeFile>,
    pub on_progress: Option<OnScanProgress>,
    pub cancel: Option<CancellationToken>,
}

/// A file in a folder by its path inside it, as path.join(folder.path, ...rel.split("/")).
pub(crate) fn in_folder(folder: &LibraryFolder, rel: &str) -> PathBuf {
    rel.split('/')
        .fold(PathBuf::from(&folder.path), |path, part| path.join(part))
}

/// Scans a folder, measuring only files that aren't in its cache as they are now, and saves the cache.
pub async fn scan_folder(
    folder: &LibraryFolder,
    options: &ScanOptions,
) -> anyhow::Result<FolderCache> {
    let previous: std::collections::HashMap<String, ScannedFile> =
        read_cache(&options.cache_dir, folder)
            .map(|cache| cache.files)
            .unwrap_or_default()
            .into_iter()
            .map(|file| (file.rel.clone(), file))
            .collect();
    let (root, kind) = (PathBuf::from(&folder.path), folder.kind);
    let (names, truncated) =
        tokio::task::spawn_blocking(move || list_folder(&root, kind)).await??;
    let result: Mutex<Vec<Option<ScannedFile>>> = Mutex::new(vec![None; names.len()]);
    let done = AtomicUsize::new(0);
    let next = AtomicUsize::new(0);
    let aborted = || {
        options
            .cancel
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
    };
    let progress = |done: usize| {
        if let Some(on_progress) = &options.on_progress {
            on_progress(done, names.len());
        }
    };
    progress(0);

    let scan_one = async |index: usize| -> Option<ScannedFile> {
        let rel = &names[index];
        let file = in_folder(folder, rel);
        let stat = tokio::fs::metadata(&file).await.ok()?;
        let (size, mtime_ms) = (stat.len(), js::mtime_ms(&stat));
        if let Some(cached) = previous.get(rel)
            && cached.size == size
            && cached.mtime_ms == mtime_ms
            && cached.measured
        {
            return Some(cached.clone());
        }
        let (probe, loudness) = measuring(folder.kind, rel);
        let mut measured = Probe::default();
        let mut ok = true;
        if probe {
            match options.probe.probe(&file, loudness).await {
                Ok(probe) => measured = probe,
                Err(_) => ok = false,
            }
        }
        Some(ScannedFile {
            rel: rel.clone(),
            size,
            mtime_ms,
            duration: measured.duration,
            channels: measured.channels,
            loudness: measured.loudness,
            measured: ok,
        })
    };
    let worker = async || {
        loop {
            if aborted() {
                return;
            }
            let index = next.fetch_add(1, Ordering::SeqCst);
            if index >= names.len() {
                return;
            }
            let scanned = scan_one(index).await;
            result.lock().unwrap()[index] = scanned;
            progress(done.fetch_add(1, Ordering::SeqCst) + 1);
        }
    };
    let workers: Vec<_> = (0..PARALLEL).map(|_| worker()).collect();
    join_all(workers).await;
    if aborted() {
        bail!("The scan was stopped.");
    }

    let cache = FolderCache {
        version: VERSION,
        path: folder.path.clone(),
        scanned_at: js::iso_now(),
        files: result.into_inner().unwrap().into_iter().flatten().collect(),
        truncated,
    };
    write_cache(&options.cache_dir, &folder.id, &cache)?;
    Ok(cache)
}

/// Runs futures side by side on this task, like Promise.all.
pub(crate) async fn join_all<F: Future<Output = ()>>(futures: Vec<F>) {
    let mut futures: Vec<_> = futures.into_iter().map(Box::pin).collect();
    std::future::poll_fn(|cx| {
        futures.retain_mut(|future| future.as_mut().poll(cx).is_pending());
        if futures.is_empty() {
            std::task::Poll::Ready(())
        } else {
            std::task::Poll::Pending
        }
    })
    .await;
}

fn channel_layout(layout: &str) -> Option<u32> {
    Some(match layout {
        "mono" => 1,
        "stereo" => 2,
        "2.1" => 3,
        "quad" | "4.0" => 4,
        "5.0" => 5,
        "5.1" => 6,
        "6.1" => 7,
        "7.1" => 8,
        _ => return None,
    })
}

/// The length in seconds from "Duration: 00:03:21.50"; None when ffmpeg didn't say.
pub(crate) fn parse_duration(log: &str) -> Option<f64> {
    static TIME: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"Duration: ([0-9]+):([0-9]+):([0-9.]+)").unwrap());
    let time = TIME.captures(log)?;
    Some(
        js::parse_number(&time[1]) * 3600.0
            + js::parse_number(&time[2]) * 60.0
            + js::parse_number(&time[3]),
    )
}

/// What ffmpeg says about the input, before it describes the output.
pub(crate) fn input_part(log: &str) -> &str {
    static OUTPUT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\nOutput #0|\nStream mapping:").unwrap());
    OUTPUT.split(log).next().unwrap_or(log)
}

/// The integrated loudness ebur128 ends its log with.
pub(crate) fn parse_loudness(log: &str) -> Option<f64> {
    static INTEGRATED: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"Integrated loudness:\s*\n\s*I:\s*(-?[0-9.]+) LUFS").unwrap());
    let value = js::parse_number(&INTEGRATED.captures(log)?[1]);
    value.is_finite().then_some(value)
}

/// Duration, channels and loudness from ffmpeg's log; None for what it doesn't say.
pub fn parse_probe(log: &str) -> Probe {
    static AUDIO_STREAM: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"Stream #[0-9]+:[0-9]+[^:\n]*: Audio: [^\n]*?, [0-9]+ Hz, ([^,\n]+)").unwrap()
    });
    static PARENS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?R)\(.*\)$").unwrap());
    static COUNT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([0-9]+) channels?$").unwrap());
    let duration = parse_duration(log);
    // The input's first audio stream: "Audio: pcm_s16le, 48000 Hz, stereo, s16" or "…, 2 channels, …".
    let channels = AUDIO_STREAM.captures(input_part(log)).and_then(|audio| {
        let layout = PARENS.replace(js::trim(&audio[1]), "").into_owned();
        match COUNT.captures(&layout) {
            Some(count) => count[1].parse().ok(),
            None => channel_layout(&layout),
        }
    });
    Probe {
        duration: duration
            .filter(|duration| duration.is_finite() && *duration > 0.0)
            .map(|duration| js::round_to(duration, 3)),
        channels,
        loudness: parse_loudness(log),
    }
}

/// Finds ffmpeg when it's needed, so a missing one only fails the files that need it.
pub type FindFfmpeg = Arc<dyn Fn() -> anyhow::Result<PathBuf> + Send + Sync>;

/// Measures files with ffmpeg: its log has the duration and channels, and ebur128 adds the loudness.
pub struct FfmpegProbe {
    pub ffmpeg: FindFfmpeg,
}

pub fn ffmpeg_probe(ffmpeg: FindFfmpeg) -> FfmpegProbe {
    FfmpegProbe { ffmpeg }
}

#[async_trait]
impl ProbeFile for FfmpegProbe {
    async fn probe(&self, file: &Path, loudness: bool) -> anyhow::Result<Probe> {
        let command = (self.ffmpeg)()?;
        let file = file.as_os_str();
        // Without an output ffmpeg stops after reading the header, which is all the length and channels need.
        let mut args: Vec<&OsStr> = vec![OsStr::new("-hide_banner")];
        if loudness {
            args.push(OsStr::new("-nostats"));
        }
        args.extend([OsStr::new("-i"), file]);
        if loudness {
            args.extend(
                [
                    "-map",
                    "0:a:0",
                    "-af",
                    "ebur128=framelog=verbose",
                    "-f",
                    "null",
                    "-",
                ]
                .map(OsStr::new),
            );
        }
        // A file ffmpeg can't read gets nothing measured, and isn't tried again until it changes.
        let output = run_process(&command, &args, Duration::from_secs(5 * 60), None).await?;
        Ok(parse_probe(&output.log))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::testing::{
        counting_probe, folder, link, temp, test_ffmpeg, wav, write, write_bytes,
    };

    fn rels(cache: &FolderCache) -> Vec<&str> {
        cache.files.iter().map(|file| file.rel.as_str()).collect()
    }

    fn options(cache_dir: &Path, probe: Arc<dyn ProbeFile>) -> ScanOptions {
        ScanOptions {
            cache_dir: cache_dir.to_path_buf(),
            probe,
            on_progress: None,
            cancel: None,
        }
    }

    #[tokio::test]
    async fn measures_new_and_changed_files_only_and_keeps_the_rest_from_its_cache() {
        let dir = temp();
        let cache_dir = temp();
        let dir_path = dir.path().to_string_lossy().into_owned();
        write(&dir.path().join("Upbeat.wav"), "x");
        write(&dir.path().join("Calm").join("Piano.mp3"), "x");
        write(&dir.path().join("cover.jpg"), "x");
        write(&dir.path().join(".hidden.wav"), "x");
        let probe = counting_probe();
        let music = folder(&dir_path, "music", "music");
        let options = options(cache_dir.path(), probe.clone());

        let first = scan_folder(&music, &options).await.unwrap();
        assert_eq!(rels(&first), ["Calm/Piano.mp3", "Upbeat.wav"]);
        assert_eq!(
            (first.files[0].duration, first.files[0].channels),
            (Some(12.5), Some(2))
        );
        assert_eq!(first.files[0].loudness, Some(-14.0));
        assert!(first.files[0].measured);
        assert_eq!(probe.calls_sorted(), ["Piano.mp3", "Upbeat.wav"]);

        probe.clear();
        let again = scan_folder(&music, &options).await.unwrap();
        assert!(probe.calls_sorted().is_empty());
        assert_eq!(again.files, first.files);

        write(&dir.path().join("Upbeat.wav"), "changed, so longer");
        write(&dir.path().join("New.wav"), "x");
        std::fs::remove_dir_all(dir.path().join("Calm")).unwrap();
        let third = scan_folder(&music, &options).await.unwrap();
        assert_eq!(probe.calls_sorted(), ["New.wav", "Upbeat.wav"]);
        assert_eq!(rels(&third), ["New.wav", "Upbeat.wav"]);
        assert_eq!(
            rels(&read_cache(cache_dir.path(), &music).unwrap()),
            ["New.wav", "Upbeat.wav"]
        );

        // A folder that now points elsewhere starts over.
        let elsewhere = temp();
        let moved = LibraryFolder {
            path: elsewhere.path().to_string_lossy().into_owned(),
            ..music
        };
        assert_eq!(read_cache(cache_dir.path(), &moved), None);
    }

    #[tokio::test]
    async fn measures_again_what_ffmpeg_wasnt_there_for() {
        struct Broken(AtomicUsize);
        #[async_trait]
        impl ProbeFile for Broken {
            async fn probe(&self, _: &Path, _: bool) -> anyhow::Result<Probe> {
                self.0.fetch_add(1, Ordering::SeqCst);
                bail!("ffmpeg is missing.")
            }
        }
        let dir = temp();
        let cache_dir = temp();
        write(&dir.path().join("Song.wav"), "x");
        let broken = Arc::new(Broken(AtomicUsize::new(0)));
        let options = options(cache_dir.path(), broken.clone());
        let music = folder(&dir.path().to_string_lossy(), "music", "music");
        let first = scan_folder(&music, &options).await.unwrap();
        assert_eq!(first.files[0].duration, None);
        assert!(!first.files[0].measured);
        scan_folder(&music, &options).await.unwrap();
        assert_eq!(broken.0.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn lists_only_the_files_of_the_folders_kind_and_doesnt_follow_links_out_of_it() {
        let dir = temp();
        let outside = temp();
        write(&dir.path().join("Logo.png"), "x");
        write(&dir.path().join("Lower third.mov"), "x");
        write(&dir.path().join("Song.wav"), "x");
        write(&dir.path().join("Teal.cube"), "x");
        write(&outside.path().join("Secret.png"), "x");
        assert!(link(outside.path(), &dir.path().join("Linked"), true));
        let dir_path = dir.path().to_string_lossy().into_owned();
        let scan = async |kind: &str| {
            let cache_dir = temp();
            let cache = scan_folder(
                &folder(&dir_path, kind, kind),
                &options(cache_dir.path(), counting_probe()),
            )
            .await
            .unwrap();
            rels(&cache)
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        };
        assert_eq!(scan("graphics").await, ["Logo.png", "Lower third.mov"]);
        assert_eq!(scan("luts").await, ["Teal.cube"]);
        assert_eq!(
            scan("other").await,
            ["Logo.png", "Lower third.mov", "Song.wav", "Teal.cube"]
        );
    }

    #[test]
    fn reads_duration_channels_and_loudness_from_ffmpegs_log() {
        let log = "Input #0, wav, from 'song.wav':
  Duration: 00:03:21.50, bitrate: 1411 kb/s
  Stream #0:0: Audio: pcm_s16le ([1][0][0][0] / 0x0001), 44100 Hz, 2 channels, s16, 1411 kb/s
Stream mapping:
  Stream #0:0 -> #0:0 (pcm_s16le (native) -> pcm_s16le (native))
Output #0, null, to 'pipe:':
  Stream #0:0: Audio: pcm_s16le, 44100 Hz, mono, s16, 1411 kb/s
[Parsed_ebur128_0 @ 000002230e3b63c0] Summary:

  Integrated loudness:
    I:         -21.8 LUFS
    Threshold: -31.8 LUFS";
        assert_eq!(
            parse_probe(log),
            Probe {
                duration: Some(201.5),
                channels: Some(2),
                loudness: Some(-21.8)
            }
        );
        assert_eq!(
            parse_probe(
                "  Duration: 00:00:10.00, start: 0\n  Stream #0:1(und): Audio: aac (LC), 48000 Hz, 5.1(side), fltp"
            ),
            Probe {
                duration: Some(10.0),
                channels: Some(6),
                loudness: None
            }
        );
        assert_eq!(
            parse_probe("  Duration: N/A, bitrate: N/A\n  Stream #0:0: Video: png, rgba, 640x360"),
            Probe::default()
        );
    }

    #[tokio::test]
    async fn measures_a_real_file_with_the_ffmpeg_linker_edit_ships() {
        let Some(ffmpeg) = test_ffmpeg() else {
            return;
        };
        let dir = temp();
        let tone = dir.path().join("Tone.wav");
        write_bytes(&tone, &wav(2.0, 2, 8000));
        let probe = ffmpeg_probe(Arc::new(move || Ok(ffmpeg.clone())));
        let measured = probe.probe(&tone, true).await.unwrap();
        assert_eq!(measured.duration, Some(2.0));
        assert_eq!(measured.channels, Some(2));
        let loudness = measured.loudness.unwrap();
        assert!(loudness < -5.0 && loudness > -40.0, "{loudness}");
        assert_eq!(
            probe.probe(&tone, false).await.unwrap(),
            Probe {
                duration: Some(2.0),
                channels: Some(2),
                loudness: None
            }
        );
    }

    #[test]
    fn writes_the_cache_as_the_typescript_did() {
        let cache = FolderCache {
            version: 1,
            path: "/music".into(),
            scanned_at: "2026-11-18T10:00:00.000Z".into(),
            files: vec![ScannedFile {
                rel: "a.wav".into(),
                size: 3,
                mtime_ms: 1_763_459_200_123.25,
                duration: Some(30.0),
                channels: Some(2),
                loudness: None,
                measured: true,
            }],
            truncated: false,
        };
        assert_eq!(
            serde_json::to_string(&cache).unwrap(),
            r#"{"version":1,"path":"/music","scannedAt":"2026-11-18T10:00:00.000Z","files":[{"rel":"a.wav","size":3,"mtimeMs":1763459200123.25,"duration":30,"channels":2,"loudness":null,"measured":true}]}"#
        );
    }
}
