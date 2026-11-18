// Finding ffmpeg, and finding or downloading whisper.cpp and its models: pinned builds, checked by hash.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhisperTools {
    pub ffmpeg: PathBuf,
    pub whisper_cli: PathBuf,
    pub model: PathBuf,
    pub vad_model: PathBuf,
    /// Names the engine in each transcript.
    pub label: String,
}

/// What setup is doing, for the status line: a download with its percent, or None while unpacking.
pub type SetupProgress = Arc<dyn Fn(&str, Option<f64>) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Download {
    pub url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
}

// Pinned builds and models, checked by hash, so every machine runs the same tested engine.
const WHISPER_BUILD: &str = "b5454";
const WINDOWS_CUDA: Download = Download {
    url: "https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-win-cuda-12.4.0-x64.zip",
    sha256: "afef0b881c500958921c3f5523b50e59ee2ec9b6f5cbd25b324c51ed308a957a",
    size: 684_913_404,
};
const WINDOWS_CPU: Download = Download {
    url: "https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-x64.zip",
    sha256: "6ba69e3482d7826214f90a6a9c84ca07782aec1e1d0c6a7c30c994fd5d816ccb",
    size: 8_928_640,
};
// CPU builds, which run on any recent glibc. A whisper-cli on PATH, such as a GPU build, comes first.
const LINUX_X64: Download = Download {
    url: "https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-ubuntu-x64.tar.gz",
    sha256: "a72becf15d7917f990f6313867a52638b82b7f9ef237fb0c980dac56a135781c",
    size: 10_364_195,
};
const LINUX_ARM64: Download = Download {
    url: "https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-ubuntu-arm64.tar.gz",
    sha256: "6b95ebfc60447df48e70ef00a73bdc3f41679ed2d01ef465827206a2ff325149",
    size: 4_608_377,
};

const MODEL_NAME: &str = "large-v3-turbo";
const MODEL: Download = Download {
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-large-v3-turbo-q8_0.bin",
    sha256: "317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1",
    size: 874_188_075,
};
const VAD_MODEL: Download = Download {
    url: "https://huggingface.co/ggml-org/whisper-vad/resolve/9ffd54a1e1ee413ddf265af9913beaf518d1639b/ggml-silero-v5.1.2.bin",
    sha256: "29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf",
    size: 885_098,
};

/// The Linux build for this processor, and Node's name for it (process.arch), which is in its folder's name.
fn linux_build() -> Option<(Download, &'static str)> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    match std::env::consts::ARCH {
        "x86_64" => Some((LINUX_X64, "x64")),
        "aarch64" => Some((LINUX_ARM64, "arm64")),
        _ => None,
    }
}

const WINDOWS_X64: bool = cfg!(all(windows, target_arch = "x86_64"));

// --- Processes ---

/// A child process as Linker-Edit starts them: no console window on Windows, killed when dropped.
pub(crate) fn command(program: impl AsRef<OsStr>) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    command.kill_on_drop(true);
    crate::js::hide_window(&mut command);
    command
}

/// How a program ended, and what it wrote to stderr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunOutput {
    /// None when it was stopped: the time ran out, or the cancel token fired.
    pub code: Option<i32>,
    pub log: String,
}

/// Runs a program, keeping its stderr (ffmpeg's log). An error only when it couldn't be started.
pub(crate) async fn run_process(
    program: &Path,
    args: &[&OsStr],
    timeout: Duration,
    cancel: Option<&CancellationToken>,
) -> std::io::Result<RunOutput> {
    let mut child = command(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let reading = tokio::spawn(async move {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes).await;
        bytes
    });
    let never = CancellationToken::new();
    let cancel = cancel.unwrap_or(&never);
    let code = tokio::select! {
        status = child.wait() => status?.code(),
        () = tokio::time::sleep(timeout) => None,
        () = cancel.cancelled() => None,
    };
    if code.is_none() {
        let _ = child.kill().await;
    }
    let bytes = reading.await.unwrap_or_default();
    Ok(RunOutput {
        code,
        log: String::from_utf8_lossy(&bytes).into_owned(),
    })
}

/// Does the program run and exit with 0 within 10 seconds?
async fn succeeds(program: &Path, args: &[&str]) -> bool {
    let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    run_process(program, &args, Duration::from_secs(10), None)
        .await
        .is_ok_and(|output| output.code == Some(0))
}

// --- Finding programs ---

pub fn find_on_path(name: &str) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{name}.exe"), name.to_string()]
    } else {
        vec![name.to_string()]
    };
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        for candidate in &names {
            let file = dir.join(candidate);
            if file.exists() {
                return Some(file);
            }
        }
    }
    None
}

const FFMPEG: &str = if cfg!(windows) {
    "ffmpeg.exe"
} else {
    "ffmpeg"
};

/// ffmpeg: the one given, else the one Linker-Edit ships next to its program (`<exe dir>/ffmpeg.exe` on Windows,
/// `<exe dir>/ffmpeg` elsewhere), else one on PATH. A debug build from source last tries the one `npm ci` puts in
/// the repo's node_modules (ffmpeg-static), as the TypeScript server did.
pub fn find_ffmpeg_in(explicit: Option<&Path>) -> anyhow::Result<PathBuf> {
    let bundled = || {
        let exe = std::env::current_exe().ok()?;
        Some(exe.parent()?.join(FFMPEG)).filter(|file| file.is_file())
    };
    let npm = || {
        let file = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../node_modules/ffmpeg-static")
            .join(FFMPEG);
        (cfg!(debug_assertions) && file.is_file()).then_some(file)
    };
    explicit
        .map(Path::to_path_buf)
        .filter(|file| file.is_file())
        .or_else(bundled)
        .or_else(|| find_on_path("ffmpeg"))
        .or_else(npm)
        .ok_or_else(|| {
            anyhow!(
                "ffmpeg is missing. Reinstall Linker-Edit, or install ffmpeg so it's on your PATH."
            )
        })
}

/// The ffmpeg Linker-Edit ships, or one on PATH: [`find_ffmpeg_in`] with none given.
pub fn find_ffmpeg() -> anyhow::Result<PathBuf> {
    find_ffmpeg_in(None)
}

// --- Downloads ---

/// Downloads to a .part file and only keeps it when the hash matches, so a broken download is retried.
pub async fn download(
    item: &Download,
    target: &Path,
    on_percent: &(dyn Fn(f64) + Send + Sync),
    cancel: Option<&CancellationToken>,
) -> anyhow::Result<()> {
    if target.exists() {
        return Ok(());
    }
    if let Some(dir) = target.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let never = CancellationToken::new();
    let cancel = cancel.unwrap_or(&never);
    let stopped = || anyhow!("The download was stopped.");
    let response = tokio::select! {
        response = reqwest::Client::new().get(item.url).send() => response?,
        () = cancel.cancelled() => return Err(stopped()),
    };
    if !response.status().is_success() {
        bail!(
            "Download failed ({}): {}",
            response.status().as_u16(),
            item.url
        );
    }
    let part = PathBuf::from(format!("{}.part", target.display()));
    let written = async {
        let mut response = response;
        let mut out = tokio::fs::File::create(&part).await?;
        let mut hash = Sha256::new();
        let mut received: u64 = 0;
        let mut reported = -1.0;
        loop {
            let chunk = tokio::select! {
                chunk = response.chunk() => chunk?,
                () = cancel.cancelled() => return Err(stopped()),
            };
            let Some(chunk) = chunk else {
                break;
            };
            hash.update(&chunk);
            received += chunk.len() as u64;
            out.write_all(&chunk).await?;
            let percent = ((received as f64 / item.size as f64) * 100.0).floor();
            if percent != reported {
                reported = percent;
                on_percent(percent);
            }
        }
        out.flush().await?;
        Ok::<_, anyhow::Error>(hash.finalize())
    }
    .await;
    let digest = match written {
        Ok(digest) => digest,
        Err(error) => {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(error);
        }
    };
    if crate::net::hex(&digest) != item.sha256 {
        let _ = tokio::fs::remove_file(&part).await;
        bail!(
            "The download of {} was damaged. Try again.",
            target.file_name().unwrap_or_default().to_string_lossy()
        );
    }
    tokio::fs::rename(&part, target).await?;
    Ok(())
}

/// Unpacks into a temporary folder first, so a half-unpacked engine is never taken for a working one.
async fn unpack(archive: &Path, home: &Path, tar: &Path, flags: &str) -> anyhow::Result<()> {
    let unpacking = PathBuf::from(format!("{}.unpacking", home.display()));
    let _ = std::fs::remove_dir_all(&unpacking);
    std::fs::create_dir_all(&unpacking)?;
    let archive_arg = archive.as_os_str().to_os_string();
    let unpacking_arg = unpacking.as_os_str().to_os_string();
    let args = [
        OsStr::new(flags),
        &archive_arg,
        OsStr::new("-C"),
        &unpacking_arg,
    ];
    let unpacked = run_process(tar, &args, Duration::from_secs(10), None)
        .await
        .is_ok_and(|output| output.code == Some(0));
    if !unpacked {
        bail!("Couldn't unpack {}.", archive.display());
    }
    std::fs::rename(&unpacking, home)?;
    let _ = std::fs::remove_file(archive);
    Ok(())
}

fn model_file(dir: &Path, item: &Download) -> PathBuf {
    dir.join("models")
        .join(item.url.rsplit('/').next().unwrap_or(item.url))
}

fn windows_whisper_cli(dir: &Path, kind: &str) -> PathBuf {
    dir.join(format!("whisper.cpp-{WHISPER_BUILD}-{kind}"))
        .join("Release")
        .join("whisper-cli.exe")
}

async fn bundled_windows_whisper(
    dir: &Path,
    on_progress: &SetupProgress,
    cancel: &CancellationToken,
) -> anyhow::Result<(PathBuf, bool)> {
    let gpu = succeeds(Path::new("nvidia-smi"), &["-L"]).await;
    let kind = if gpu { "cuda" } else { "cpu" };
    let cli = windows_whisper_cli(dir, kind);
    if cli.exists() {
        return Ok((cli, gpu));
    }
    let home = dir.join(format!("whisper.cpp-{WHISPER_BUILD}-{kind}"));
    let zip = PathBuf::from(format!("{}.zip", home.display()));
    let build = if gpu { WINDOWS_CUDA } else { WINDOWS_CPU };
    download(
        &build,
        &zip,
        &|percent| on_progress("Downloading the speech engine", Some(percent)),
        Some(cancel),
    )
    .await?;
    on_progress("Unpacking the speech engine", None);
    // Windows' own tar reads zip files; a Git Bash tar earlier on PATH does not.
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let tar = PathBuf::from(root).join("System32").join("tar.exe");
    unpack(&zip, &home, &tar, "-xf").await?;
    Ok((cli, gpu))
}

/// Where the downloaded Linux engine's whisper-cli is, or None when there's no build for this processor.
fn linux_whisper_cli(dir: &Path) -> Option<PathBuf> {
    let (_, arch) = linux_build()?;
    let home = dir.join(format!("whisper.cpp-{WHISPER_BUILD}-linux-{arch}"));
    Some(
        home.join(format!("whisper-bin-ubuntu-{arch}"))
            .join("whisper-cli"),
    )
}

async fn bundled_linux_whisper(
    dir: &Path,
    on_progress: &SetupProgress,
    cancel: &CancellationToken,
) -> anyhow::Result<Option<PathBuf>> {
    let (Some(cli), Some((build, _))) = (linux_whisper_cli(dir), linux_build()) else {
        return Ok(None);
    };
    if cli.exists() {
        return Ok(Some(cli));
    }
    let home = cli
        .parent()
        .and_then(Path::parent)
        .context("whisper-cli has a folder")?
        .to_path_buf();
    let archive = PathBuf::from(format!("{}.tar.gz", home.display()));
    download(
        &build,
        &archive,
        &|percent| on_progress("Downloading the speech engine", Some(percent)),
        Some(cancel),
    )
    .await?;
    on_progress("Unpacking the speech engine", None);
    unpack(&archive, &home, Path::new("tar"), "-xzf").await?;
    Ok(Some(cli))
}

const MISSING_WHISPER: &str = if cfg!(target_os = "macos") {
    "Install whisper.cpp with `brew install whisper-cpp`, or set whisperCli in linker-edit.config.json."
} else {
    "Install whisper.cpp so `whisper-cli` is on PATH, or set whisperCli in linker-edit.config.json."
};

/// Is everything prepare_tools needs already here, so a first transcription starts without downloading?
pub fn tools_ready(dir: &Path, whisper_override: Option<&Path>) -> bool {
    if find_ffmpeg().is_err() {
        return false;
    }
    let whisper = match whisper_override {
        Some(file) => file.exists(),
        None if WINDOWS_X64 => ["cuda", "cpu"]
            .iter()
            .any(|kind| windows_whisper_cli(dir, kind).exists()),
        None => {
            find_on_path("whisper-cli").is_some()
                || linux_whisper_cli(dir).is_some_and(|cli| cli.exists())
        }
    };
    whisper
        && [MODEL, VAD_MODEL]
            .iter()
            .all(|item| model_file(dir, item).exists())
}

/// Finds or downloads ffmpeg, whisper.cpp and its models. The first run downloads up to 1.6 GB.
pub async fn prepare_tools(
    dir: &Path,
    whisper_override: Option<&Path>,
    on_progress: SetupProgress,
    cancel: CancellationToken,
) -> anyhow::Result<WhisperTools> {
    let ffmpeg = find_ffmpeg()?;
    let mut whisper_cli = whisper_override.map(Path::to_path_buf);
    let mut device = "";
    if whisper_cli.is_none() && WINDOWS_X64 {
        let (cli, gpu) = bundled_windows_whisper(dir, &on_progress, &cancel).await?;
        whisper_cli = Some(cli);
        device = if gpu { " (CUDA)" } else { " (CPU)" };
    }
    if whisper_cli.is_none() {
        whisper_cli = find_on_path("whisper-cli");
    }
    if whisper_cli.is_none() {
        whisper_cli = bundled_linux_whisper(dir, &on_progress, &cancel).await?;
        if whisper_cli.is_some() {
            device = " (CPU)";
        }
    }
    let Some(whisper_cli) = whisper_cli.filter(|cli| cli.exists()) else {
        bail!("whisper.cpp not found. {MISSING_WHISPER}");
    };

    let model = model_file(dir, &MODEL);
    let vad_model = model_file(dir, &VAD_MODEL);
    download(
        &MODEL,
        &model,
        &|percent| on_progress("Downloading the speech model", Some(percent)),
        Some(&cancel),
    )
    .await?;
    download(
        &VAD_MODEL,
        &vad_model,
        &|percent| on_progress("Downloading the voice detector", Some(percent)),
        Some(&cancel),
    )
    .await?;
    Ok(WhisperTools {
        ffmpeg,
        whisper_cli,
        model,
        vad_model,
        label: format!("whisper.cpp {MODEL_NAME}{device}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::testing::temp;

    #[test]
    fn keeps_the_typescripts_download_folders_and_file_names() {
        let dir = Path::new("tools");
        assert_eq!(
            model_file(dir, &MODEL),
            dir.join("models").join("ggml-large-v3-turbo-q8_0.bin")
        );
        assert_eq!(
            windows_whisper_cli(dir, "cuda"),
            dir.join("whisper.cpp-b5454-cuda")
                .join("Release")
                .join("whisper-cli.exe")
        );
        let cli = Path::new("/nowhere/whisper-cli");
        assert!(!tools_ready(&temp().path().join("empty"), Some(cli)));
    }

    #[tokio::test]
    async fn keeps_nothing_of_a_download_that_cant_start() {
        let dir = temp();
        let target = dir.path().join("models").join("model.bin");
        let broken = Download {
            url: "http://127.0.0.1:9/model.bin",
            sha256: "",
            size: 1,
        };
        assert!(download(&broken, &target, &|_| {}, None).await.is_err());
        assert!(!target.exists());
        assert!(!PathBuf::from(format!("{}.part", target.display())).exists());
        // Already there: nothing to download.
        std::fs::write(&target, "x").unwrap();
        download(&broken, &target, &|_| {}, None).await.unwrap();
    }

    #[tokio::test]
    async fn runs_ffmpeg_and_keeps_its_log() {
        let Some(ffmpeg) = crate::library::testing::test_ffmpeg() else {
            return;
        };
        let output = run_process(
            &ffmpeg,
            &[
                OsStr::new("-hide_banner"),
                OsStr::new("-i"),
                OsStr::new("missing.wav"),
            ],
            Duration::from_secs(30),
            None,
        )
        .await
        .unwrap();
        assert_ne!(output.code, Some(0));
        assert!(output.log.contains("missing.wav"), "{}", output.log);
        assert!(
            run_process(
                Path::new("no-such-program-here"),
                &[],
                Duration::from_secs(5),
                None
            )
            .await
            .is_err()
        );
    }
}
