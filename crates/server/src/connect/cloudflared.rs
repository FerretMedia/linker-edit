// cloudflared keeps this computer's Linker Connect tunnel open, as it does for T3 Connect. Pinned and checked by
// hash like the speech engine, so every computer runs the same tested release. One the user installed (on PATH, or
// `cloudflared` in the config) is their choice and comes first.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use anyhow::bail;
use async_trait::async_trait;
use regex::Regex;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::Notify;

use crate::js::hide_window;
use crate::transcribe::tools::{Download, download, find_on_path};

// The version is a macro so the download addresses below can be built with concat! at compile time.
macro_rules! cloudflared_version {
    () => {
        "2026.10.0"
    };
}

macro_rules! release {
    ($file:literal) => {
        concat!(
            "https://github.com/cloudflare/cloudflared/releases/download/",
            cloudflared_version!(),
            "/",
            $file
        )
    };
}

pub const CLOUDFLARED_VERSION: &str = cloudflared_version!();

/// A pinned download, checked against its SHA-256 (GitHub's asset digest).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Build {
    pub download: Download,
    /// A .tgz holding cloudflared (macOS), not the program itself.
    pub tgz: bool,
}

/// By platform and processor, in Node's names. Windows on Arm runs the x64 build.
const BUILDS: [(&str, Build); 5] = [
    (
        "win32-x64",
        Build {
            download: Download {
                url: release!("cloudflared-windows-amd64.exe"),
                sha256: "86aee4017b26625cee8484c113558f48effa4cd47f7aa05fcf425604e5d2b23c",
                size: 55_365_048,
            },
            tgz: false,
        },
    ),
    (
        "linux-x64",
        Build {
            download: Download {
                url: release!("cloudflared-linux-amd64"),
                sha256: "d33ff2d14475178d2012c2c56beba87389ac5ded27649519f198a7d3134a99db",
                size: 40_129_756,
            },
            tgz: false,
        },
    ),
    (
        "linux-arm64",
        Build {
            download: Download {
                url: release!("cloudflared-linux-arm64"),
                sha256: "e6422b9d4f72d3194bc5a38676f13667c06666523217b842a877d72a80b5ac08",
                size: 37_687_584,
            },
            tgz: false,
        },
    ),
    (
        "darwin-x64",
        Build {
            download: Download {
                url: release!("cloudflared-darwin-amd64.tgz"),
                sha256: "903845b81828c8cb3c5d13d816a2de71c06a3da5785469df8eb0e1b736d92f9f",
                size: 21_741_581,
            },
            tgz: true,
        },
    ),
    (
        "darwin-arm64",
        Build {
            download: Download {
                url: release!("cloudflared-darwin-arm64.tgz"),
                sha256: "a2f79ff7b9420aa537d74af239f376da170bbabeb529aec416002adac6a72e70",
                size: 19_809_074,
            },
            tgz: true,
        },
    ),
];

/// The build for a platform and processor in Node's names (process.platform, process.arch).
pub fn build_for(platform: &str, arch: &str) -> Option<Build> {
    let key = if platform == "win32" {
        "win32-x64".to_string()
    } else {
        format!("{platform}-{arch}")
    };
    BUILDS
        .into_iter()
        .find(|(name, _)| *name == key)
        .map(|(_, build)| build)
}

/// The build for this computer.
fn this_build() -> Option<Build> {
    let platform = match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "ia32",
        other => other,
    };
    build_for(platform, arch)
}

pub type OnPercent = Arc<dyn Fn(u32) + Send + Sync>;

/// Finds or downloads cloudflared, about 20 to 55 MB once, into dir (AppConfig::tools_dir). Answers the file to run.
pub async fn prepare_cloudflared(
    dir: &Path,
    override_file: Option<&Path>,
    on_percent: OnPercent,
) -> anyhow::Result<PathBuf> {
    if let Some(file) = override_file {
        if !file.exists() {
            bail!(
                "cloudflared isn't at {}. Fix \"cloudflared\" in linker-edit.config.json.",
                file.display()
            );
        }
        return Ok(file.to_path_buf());
    }
    if let Some(file) = find_on_path("cloudflared") {
        return Ok(file);
    }
    let Some(build) = this_build() else {
        bail!(
            "There's no cloudflared for this computer. Install it so it's on PATH, then try again."
        );
    };
    let home = dir.join(format!("cloudflared-{CLOUDFLARED_VERSION}"));
    let file = home.join(if cfg!(windows) {
        "cloudflared.exe"
    } else {
        "cloudflared"
    });
    if file.exists() {
        return Ok(file);
    }
    let on_percent = move |percent: f64| on_percent(percent as u32);
    if !build.tgz {
        download(&build.download, &file, &on_percent, None).await?;
    } else {
        let archive = dir.join(format!("cloudflared-{CLOUDFLARED_VERSION}.tgz"));
        download(&build.download, &archive, &on_percent, None).await?;
        let mut unpacking = home.as_os_str().to_owned();
        unpacking.push(".unpacking");
        let unpacking = PathBuf::from(unpacking);
        let _ = std::fs::remove_dir_all(&unpacking);
        std::fs::create_dir_all(&unpacking)?;
        let unpacked = tokio::process::Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(&unpacking)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .is_ok_and(|status| status.success());
        if !unpacked {
            bail!("Couldn't unpack {}.", archive.display());
        }
        std::fs::rename(&unpacking, &home)?;
        let _ = std::fs::remove_file(&archive);
    }
    #[cfg(unix)]
    {
        use anyhow::Context;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755))
            .context("Couldn't make cloudflared runnable")?;
    }
    Ok(file)
}

/// Finds or downloads cloudflared for Linker Connect; tests use a stand-in.
#[async_trait]
pub trait PrepareCloudflared: Send + Sync {
    async fn prepare(&self, on_percent: OnPercent) -> anyhow::Result<PathBuf>;
}

/// The real one: prepare_cloudflared into dir, or the user's own (`cloudflared` in the config).
#[derive(Debug, Clone)]
pub struct Cloudflared {
    pub dir: PathBuf,
    pub override_file: Option<PathBuf>,
}

#[async_trait]
impl PrepareCloudflared for Cloudflared {
    async fn prepare(&self, on_percent: OnPercent) -> anyhow::Result<PathBuf> {
        prepare_cloudflared(&self.dir, self.override_file.as_deref(), on_percent).await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelEvent {
    Connecting,
    Up,
    Down { detail: Option<String> },
}

/// What cloudflared's log line means for the tunnel; None for lines that change nothing.
pub fn read_log_line(line: &str) -> Option<TunnelEvent> {
    let line = line.to_lowercase();
    // As the TS: "Unregistered tunnel connection" also contains "registered tunnel connection", so it reads as up.
    if line.contains("registered tunnel connection") {
        return Some(TunnelEvent::Up);
    }
    let connecting = [
        "unregistered tunnel connection",
        "connection terminated",
        "retrying connection",
        "serve tunnel error",
    ];
    connecting
        .iter()
        .any(|text| line.contains(text))
        .then_some(TunnelEvent::Connecting)
}

/// A running tunnel.
pub trait Tunnel: Send + Sync {
    fn stop(&self);
}

pub type OnTunnelEvent = Arc<dyn Fn(TunnelEvent) + Send + Sync>;

/// Runs a tunnel; the token goes in TUNNEL_TOKEN, so it isn't on the command line other programs can read.
/// Calls on_event with Connecting before it returns, and with Down later when it stops by itself.
pub trait StartTunnel: Send + Sync {
    fn start(&self, file: &Path, token: &str, on_event: OnTunnelEvent) -> Box<dyn Tunnel>;
}

/// The real cloudflared. Must be called inside the tokio runtime.
#[derive(Debug, Clone, Copy, Default)]
pub struct CloudflaredTunnels;

struct Running {
    stopped: Arc<AtomicBool>,
    stop: Arc<Notify>,
}

impl Tunnel for Running {
    fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.stop.notify_one();
    }
}

static LOG_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\S+\s+(ERR|INF|WRN|DBG)\s+").unwrap());
static ERROR_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\sERR\s").unwrap());

/// Why it stopped: its last error, or else its last words, such as "Provided Tunnel token is not valid."
#[derive(Default)]
struct LastWords {
    error: Option<String>,
    line: Option<String>,
}

async fn read_log(
    pipe: impl AsyncRead + Unpin,
    words: Arc<Mutex<LastWords>>,
    stopped: Arc<AtomicBool>,
    on_event: OnTunnelEvent,
) {
    let mut reader = BufReader::new(pipe);
    let mut bytes = Vec::new();
    while reader.read_until(b'\n', &mut bytes).await.unwrap_or(0) > 0 {
        let raw = String::from_utf8_lossy(&bytes).into_owned();
        bytes.clear();
        let line = raw.trim_end_matches(['\n', '\r']);
        let text = LOG_PREFIX.replace(line, "").trim().to_string();
        {
            let mut words = words.lock().unwrap();
            if ERROR_LINE.is_match(line) {
                words.error = Some(text);
            } else if !text.is_empty() && !text.starts_with("See '") {
                words.line = Some(text);
            }
        }
        if let Some(event) = read_log_line(line)
            && !stopped.load(Ordering::SeqCst)
        {
            on_event(event);
        }
    }
}

impl StartTunnel for CloudflaredTunnels {
    fn start(&self, file: &Path, token: &str, on_event: OnTunnelEvent) -> Box<dyn Tunnel> {
        let mut command = tokio::process::Command::new(file);
        command
            .args(["tunnel", "--no-autoupdate", "run"])
            .env("TUNNEL_TOKEN", token)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Stops with the app, as the TS's process.once("exit", kill).
            .kill_on_drop(true);
        hide_window(&mut command);
        let spawned = command.spawn();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(Notify::new());
        on_event(TunnelEvent::Connecting);
        let running = Running {
            stopped: stopped.clone(),
            stop: stop.clone(),
        };
        tokio::spawn(async move {
            let down = |detail: String| {
                if !stopped.load(Ordering::SeqCst) {
                    on_event(TunnelEvent::Down {
                        detail: Some(detail),
                    });
                }
            };
            let mut child = match spawned {
                Ok(child) => child,
                Err(error) => return down(error.to_string()),
            };
            let words = Arc::new(Mutex::new(LastWords::default()));
            // cloudflared logs to stderr.
            let readers = [
                child.stderr.take().map(|pipe| {
                    tokio::spawn(read_log(
                        pipe,
                        words.clone(),
                        stopped.clone(),
                        on_event.clone(),
                    ))
                }),
                child.stdout.take().map(|pipe| {
                    tokio::spawn(read_log(
                        pipe,
                        words.clone(),
                        stopped.clone(),
                        on_event.clone(),
                    ))
                }),
            ];
            let status = tokio::select! {
                status = child.wait() => status,
                _ = stop.notified() => {
                    let _ = child.kill().await;
                    return;
                }
            };
            for reader in readers.into_iter().flatten() {
                let _ = reader.await;
            }
            let code = match status {
                Ok(status) => status
                    .code()
                    .map_or_else(|| "null".to_string(), |code| code.to_string()),
                Err(error) => return down(error.to_string()),
            };
            let words = std::mem::take(&mut *words.lock().unwrap());
            down(
                words
                    .error
                    .or(words.line)
                    .unwrap_or_else(|| format!("cloudflared stopped (code {code}).")),
            );
        });
        Box::new(running)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_a_pinned_build_for_every_computer_linker_edit_runs_on() {
        for (platform, arch) in [
            ("win32", "x64"),
            ("win32", "arm64"),
            ("darwin", "arm64"),
            ("darwin", "x64"),
            ("linux", "x64"),
            ("linux", "arm64"),
        ] {
            let build = build_for(platform, arch).unwrap();
            assert!(
                build
                    .download
                    .url
                    .contains(&format!("/{CLOUDFLARED_VERSION}/"))
            );
            assert_eq!(build.download.sha256.len(), 64);
            assert!(
                build
                    .download
                    .sha256
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            );
        }
        assert_eq!(build_for("linux", "ia32"), None);
        assert!(this_build().is_some());
    }

    #[test]
    fn reads_the_tunnels_state_from_its_log() {
        assert_eq!(
            read_log_line(
                "2026-11-16T10:00:00Z INF Registered tunnel connection connIndex=0 location=ams01 protocol=quic"
            ),
            Some(TunnelEvent::Up)
        );
        assert_eq!(
            read_log_line("2026-11-16T10:00:00Z WRN Retrying connection in up to 2s"),
            Some(TunnelEvent::Connecting)
        );
        assert_eq!(
            read_log_line("2026-11-16T10:00:00Z INF Starting tunnel tunnelID=abc"),
            None
        );
    }

    #[tokio::test]
    async fn says_the_tunnel_stopped_when_cloudflared_wont_start() {
        let (events, mut received) = tokio::sync::mpsc::unbounded_channel();
        let dir = tempfile::tempdir().unwrap();
        let _tunnel = CloudflaredTunnels.start(
            &dir.path().join("no-cloudflared"),
            "token",
            Arc::new(move |event| events.send(event).unwrap()),
        );
        assert_eq!(received.recv().await, Some(TunnelEvent::Connecting));
        assert!(matches!(
            received.recv().await,
            Some(TunnelEvent::Down { detail: Some(_) })
        ));
    }
}
