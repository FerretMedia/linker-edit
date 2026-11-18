// The configuration: ~/LinkerEdit/linker-edit.config.json, which the user writes by hand and Linker-Edit only reads.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{anyhow, bail};
use serde::Deserialize;
use serde_json::Value;

/// Config and workspace live here, outside the app folder npx replaces.
pub static HOME_DIR: LazyLock<PathBuf> =
    LazyLock::new(|| std::env::home_dir().unwrap_or_default().join("LinkerEdit"));

/// Optional overrides, read from linker-edit.config.json in HOME_DIR.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserConfig {
    // Numbers stay JSON values so a wrong one gets its own message below, as in the TypeScript.
    port: Option<Value>,
    /// Where new projects get their folders, and the project list lives.
    projects_dir: Option<PathBuf>,
    /// The working folder from before projects; it becomes the first project.
    workspace_dir: Option<PathBuf>,
    /// Where preview renders go. Kept out of the workspace so they never show in its Git status.
    preview_dir: Option<PathBuf>,
    /// Resolve's own MCP server (ResolveMCP), when Resolve is installed somewhere Linker-Edit doesn't look.
    resolve_mcp: Option<PathBuf>,
    /// Tokens at which Claude compacts the chat by itself, 100000 to 1000000. Claude Code decides when unset.
    auto_compact_window: Option<Value>,
    /// Where transcripts of media files are kept.
    transcripts_dir: Option<PathBuf>,
    /// Where the speech engine and its models are downloaded.
    tools_dir: Option<PathBuf>,
    /// Your own whisper-cli, instead of the one Linker-Edit downloads (Windows) or finds on PATH.
    whisper_cli: Option<PathBuf>,
    /// The HTTPS port remote access uses on your tailnet.
    tailscale_port: Option<Value>,
    /// Linker's sign-in (Clerk), for testing against staging.
    linker_auth_server: Option<String>,
    /// Linker's dashboard and API, for testing against staging.
    linker_app_url: Option<String>,
    /// An OAuth client id for signing in with Linker, instead of the one Linker publishes for Linker-Edit.
    linker_client_id: Option<String>,
    /// Your own cloudflared for Linker Connect, instead of one on PATH or the one Linker-Edit downloads.
    cloudflared: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveMcpConfig {
    /// Resolve's own MCP server program; None when this computer has none.
    pub command: Option<PathBuf>,
}

/// Signing in with a Linker account; file is where the sign-in is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkerConfig {
    pub auth_server: String,
    pub app_url: String,
    pub client_id: Option<String>,
    pub file: PathBuf,
}

/// Linker Connect: this computer's link and tunnel token, the devices signed in through it, and cloudflared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectConfig {
    pub file: PathBuf,
    pub sessions_file: PathBuf,
    pub cloudflared: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub port: u16,
    pub projects_dir: PathBuf,
    /// The agent's working folder. Each project has its own; this one becomes the first project.
    pub workspace_dir: PathBuf,
    pub preview_dir: PathBuf,
    pub transcripts_dir: PathBuf,
    pub tools_dir: PathBuf,
    pub whisper_cli: Option<PathBuf>,
    pub tailscale_port: u16,
    /// This configuration's own file, which the user writes by hand; it may not exist.
    pub config_file: PathBuf,
    /// The user's choices kept across restarts, such as supervised mode.
    pub settings_file: PathBuf,
    /// The universal library's folders, and the tags, stars and notes on library files.
    pub library_file: PathBuf,
    /// What's in each library folder, so a rescan only measures changed files.
    pub library_cache_dir: PathBuf,
    pub linker: LinkerConfig,
    pub connect: ConnectConfig,
    /// Lets the agents' MCP server call this server's API; web pages and other programs can't.
    pub api_token: String,
    pub resolve_mcp: ResolveMcpConfig,
    pub auto_compact_window: Option<u32>,
}

fn read_user_config(file: &Path) -> anyhow::Result<UserConfig> {
    if !file.exists() {
        return Ok(UserConfig::default());
    }
    let cant_read =
        |error: &dyn std::fmt::Display| anyhow!("Can't read {}: {error}", file.display());
    let text = std::fs::read_to_string(file).map_err(|error| cant_read(&error))?;
    serde_json::from_str(&text).map_err(|error| cant_read(&error))
}

/// A whole number, as Number.isInteger sees it: 4317 and 4317.0, not "4317".
fn integer(value: &Value) -> Option<f64> {
    value.as_f64().filter(|number| number.fract() == 0.0)
}

fn port_number(
    value: Option<&Value>,
    default: u16,
    name: &str,
    file: &Path,
) -> anyhow::Result<u16> {
    let Some(value) = value else {
        return Ok(default);
    };
    match integer(value) {
        Some(port) if port > 0.0 && port < 65536.0 => Ok(port as u16),
        _ => bail!("{name} in {} must be a port number.", file.display()),
    }
}

/// LINKER_EDIT_CONFIG points at another config file, so tests can run on their own port and workspace.
pub fn default_config_file() -> PathBuf {
    std::env::var_os("LINKER_EDIT_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| HOME_DIR.join("linker-edit.config.json"))
}

/// Reads the config file. find_mcp looks for Resolve's own MCP server when the config doesn't name one
/// (resolve::locate's find_resolve_mcp in the app).
pub fn load_config(
    file: &Path,
    find_mcp: impl FnOnce() -> Option<PathBuf>,
) -> anyhow::Result<AppConfig> {
    let user = read_user_config(file)?;
    let window = match &user.auto_compact_window {
        None => None,
        Some(value) => match integer(value) {
            Some(window) if (100_000.0..=1_000_000.0).contains(&window) => Some(window as u32),
            _ => bail!(
                "autoCompactWindow in {} must be a whole number from 100000 to 1000000.",
                file.display()
            ),
        },
    };
    let port = port_number(user.port.as_ref(), 4317, "port", file)?;
    let tailscale_port = port_number(user.tailscale_port.as_ref(), 8443, "tailscalePort", file)?;
    let dir = file.parent().unwrap_or(Path::new(""));
    let non_empty = |value: Option<PathBuf>| value.filter(|value| !value.as_os_str().is_empty());
    Ok(AppConfig {
        port,
        projects_dir: user
            .projects_dir
            .unwrap_or_else(|| HOME_DIR.join("projects")),
        workspace_dir: user
            .workspace_dir
            .unwrap_or_else(|| HOME_DIR.join("workspace")),
        preview_dir: user
            .preview_dir
            .unwrap_or_else(|| HOME_DIR.join("previews")),
        transcripts_dir: user
            .transcripts_dir
            .unwrap_or_else(|| HOME_DIR.join("transcripts")),
        tools_dir: user.tools_dir.unwrap_or_else(|| HOME_DIR.join("tools")),
        whisper_cli: non_empty(user.whisper_cli),
        tailscale_port,
        config_file: file.to_path_buf(),
        settings_file: dir.join("settings.json"),
        library_file: dir.join("library.json"),
        library_cache_dir: dir.join("cache").join("library"),
        linker: LinkerConfig {
            auth_server: user
                .linker_auth_server
                .unwrap_or_else(|| "https://clerk.linker.cool".into()),
            app_url: user
                .linker_app_url
                .unwrap_or_else(|| "https://app.linker.cool".into()),
            client_id: user.linker_client_id.filter(|id| !id.is_empty()),
            file: dir.join("linker-account.json"),
        },
        connect: ConnectConfig {
            file: dir.join("connect.json"),
            sessions_file: dir.join("connect-sessions.json"),
            cloudflared: non_empty(user.cloudflared),
        },
        api_token: api_token(),
        resolve_mcp: ResolveMcpConfig {
            command: user.resolve_mcp.or_else(find_mcp),
        },
        auto_compact_window: window,
    })
}

/// 24 random bytes as hex, like randomBytes(24).toString("hex").
fn api_token() -> String {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).expect("the system has randomness");
    bytes
        .iter()
        .fold(String::with_capacity(48), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// load_config on the default file, or the message and exit code 1.
pub fn load_config_or_exit(find_mcp: impl FnOnce() -> Option<PathBuf>) -> AppConfig {
    match load_config(&default_config_file(), find_mcp) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config file in a folder of its own, removed when the test ends.
    struct ConfigFile {
        _dir: tempfile::TempDir,
        file: PathBuf,
    }

    impl std::ops::Deref for ConfigFile {
        type Target = Path;
        fn deref(&self) -> &Path {
            &self.file
        }
    }

    fn config_file(contents: &str) -> ConfigFile {
        let dir = tempfile::Builder::new()
            .prefix("linker-edit-config-")
            .tempdir()
            .unwrap();
        let file = dir.path().join("linker-edit.config.json");
        std::fs::write(&file, contents).unwrap();
        ConfigFile { _dir: dir, file }
    }

    fn none() -> Option<PathBuf> {
        None
    }

    #[test]
    fn finds_resolves_own_mcp_server_unless_the_config_names_one() {
        let found = || Some(PathBuf::from("C:/Resolve/ResolveMCP.exe"));
        assert_eq!(
            load_config(&config_file("{}"), found).unwrap().resolve_mcp,
            ResolveMcpConfig {
                command: Some("C:/Resolve/ResolveMCP.exe".into())
            }
        );
        assert_eq!(
            load_config(&config_file("{}"), none).unwrap().resolve_mcp,
            ResolveMcpConfig { command: None }
        );
        let file = config_file(r#"{ "resolveMcp": "D:/Resolve/ResolveMCP.exe" }"#);
        assert_eq!(
            load_config(&file, found).unwrap().resolve_mcp,
            ResolveMcpConfig {
                command: Some("D:/Resolve/ResolveMCP.exe".into())
            }
        );
    }

    #[test]
    fn refuses_a_port_that_isnt_one() {
        let error = |json: &str| {
            load_config(&config_file(json), none)
                .unwrap_err()
                .to_string()
        };
        assert!(error(r#"{ "port": "4317" }"#).starts_with("port in "));
        assert!(error(r#"{ "port": 70000 }"#).ends_with("must be a port number."));
        assert_eq!(
            load_config(&config_file(r#"{ "port": 5000 }"#), none)
                .unwrap()
                .port,
            5000
        );
        let file = config_file(r#"{ "tailscalePort": 0 }"#);
        assert_eq!(
            load_config(&file, none).unwrap_err().to_string(),
            format!("tailscalePort in {} must be a port number.", file.display())
        );
    }

    #[test]
    fn names_the_file_when_it_isnt_valid_json() {
        let file = config_file(r#"{ "port": 4317, }"#);
        let error = load_config(&file, none).unwrap_err().to_string();
        assert!(
            error.starts_with(&format!("Can't read {}: ", file.display())),
            "{error}"
        );
    }

    #[test]
    fn takes_auto_compact_windows_in_range() {
        let file = config_file(r#"{ "autoCompactWindow": 99999 }"#);
        assert_eq!(
            load_config(&file, none).unwrap_err().to_string(),
            format!(
                "autoCompactWindow in {} must be a whole number from 100000 to 1000000.",
                file.display()
            )
        );
        assert!(load_config(&config_file(r#"{ "autoCompactWindow": 150000.5 }"#), none).is_err());
        let config = load_config(&config_file(r#"{ "autoCompactWindow": 150000 }"#), none).unwrap();
        assert_eq!(config.auto_compact_window, Some(150_000));
        assert_eq!(
            load_config(&config_file("{}"), none)
                .unwrap()
                .auto_compact_window,
            None
        );
    }

    #[test]
    fn keeps_its_files_next_to_the_config_and_its_folders_in_linker_edit() {
        let file = config_file(
            r#"{ "projectsDir": "D:\\Projects", "whisperCli": "", "cloudflared": "C:\\bin\\cloudflared.exe" }"#,
        );
        let dir = file.parent().unwrap();
        let config = load_config(&file, none).unwrap();
        assert_eq!(config.port, 4317);
        assert_eq!(config.tailscale_port, 8443);
        assert_eq!(config.projects_dir, PathBuf::from(r"D:\Projects"));
        assert_eq!(config.workspace_dir, HOME_DIR.join("workspace"));
        assert_eq!(config.preview_dir, HOME_DIR.join("previews"));
        assert_eq!(config.whisper_cli, None);
        assert_eq!(config.config_file, *file);
        assert_eq!(config.settings_file, dir.join("settings.json"));
        assert_eq!(config.library_cache_dir, dir.join("cache").join("library"));
        assert_eq!(config.linker.file, dir.join("linker-account.json"));
        assert_eq!(config.linker.auth_server, "https://clerk.linker.cool");
        assert_eq!(config.linker.client_id, None);
        assert_eq!(
            config.connect.sessions_file,
            dir.join("connect-sessions.json")
        );
        assert_eq!(
            config.connect.cloudflared,
            Some(PathBuf::from(r"C:\bin\cloudflared.exe"))
        );
        assert_eq!(config.api_token.len(), 48);
        assert_ne!(
            config.api_token,
            load_config(&file, none).unwrap().api_token
        );
    }

    #[test]
    fn a_missing_config_is_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = load_config(&dir.path().join("linker-edit.config.json"), none).unwrap();
        assert_eq!(config.port, 4317);
        assert_eq!(config.projects_dir, HOME_DIR.join("projects"));
    }
}
