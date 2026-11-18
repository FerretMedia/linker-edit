// Where DaVinci Resolve Studio 21.1.2 and newer install their own MCP server. It's Blackmagic Design's
// proprietary program: Linker-Edit starts the user's installed copy and never ships one.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

const MAC_PATHS: [&str; 2] = [
    "/Applications/DaVinci Resolve/DaVinci Resolve.app/Contents/Applications/ResolveMCP",
    "/Applications/DaVinci Resolve Studio.app/Contents/Applications/ResolveMCP",
];
const LINUX_PATH: &str = "/opt/resolve/bin/ResolveMCP";

pub const MISSING_RESOLVE_MCP: &str = "Linker-Edit drives Resolve through the MCP server that comes with DaVinci Resolve Studio 21.1.2 and newer, and can't find it. Install or update DaVinci Resolve Studio, then refresh.";

static REG_SZ: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"REG_SZ\s+"([^"]+)""#).unwrap());

/// Resolve's install folder from the .drp file association, for installs outside Program Files.
fn windows_install_dir() -> Option<PathBuf> {
    let mut command = std::process::Command::new("reg.exe");
    command.args(["query", r"HKCR\ResolveDrpFile\shell\open\command", "/ve"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = command.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let out = String::from_utf8_lossy(&out.stdout);
    let exe = REG_SZ.captures(&out)?.get(1)?.as_str().to_string();
    Some(win_dirname(&exe))
}

/// path.win32.dirname, on any platform.
fn win_dirname(file: &str) -> PathBuf {
    let cut = file.rfind(['\\', '/']).unwrap_or(0);
    PathBuf::from(&file[..cut])
}

/// path.win32.join of a folder and a file name, on any platform.
fn win_join(dir: &str, file: &str) -> PathBuf {
    PathBuf::from(format!("{}\\{file}", dir.trim_end_matches(['\\', '/'])))
}

/// What find_resolve_mcp looks at, for tests: the platform (std::env::consts::OS), %ProgramFiles%, what exists,
/// and Resolve's install folder from the registry.
pub struct LocateOptions<'a> {
    pub os: &'a str,
    pub program_files: Option<String>,
    pub exists: &'a dyn Fn(&Path) -> bool,
    pub install_dir: &'a dyn Fn() -> Option<PathBuf>,
}

/// The ResolveMCP program, or None when this Resolve doesn't have one (not installed, or older than 21.1.2).
pub fn find_resolve_mcp() -> Option<PathBuf> {
    find_resolve_mcp_with(LocateOptions {
        os: std::env::consts::OS,
        program_files: std::env::var("ProgramFiles").ok(),
        exists: &|file| file.exists(),
        install_dir: &windows_install_dir,
    })
}

pub fn find_resolve_mcp_with(options: LocateOptions) -> Option<PathBuf> {
    let exists = options.exists;
    match options.os {
        "windows" => {
            let program_files = options
                .program_files
                .as_deref()
                .unwrap_or(r"C:\Program Files");
            let usual = win_join(
                program_files,
                r"Blackmagic Design\DaVinci Resolve\ResolveMCP.exe",
            );
            if exists(&usual) {
                return Some(usual);
            }
            let registered = (options.install_dir)()
                .map(|dir| win_join(&dir.to_string_lossy(), "ResolveMCP.exe"));
            registered.filter(|file| exists(file))
        }
        "macos" => MAC_PATHS
            .iter()
            .map(PathBuf::from)
            .find(|file| exists(file)),
        _ => Some(PathBuf::from(LINUX_PATH)).filter(|file| exists(file)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(os: &str, existing: &[&str], install_dir: Option<&str>) -> Option<PathBuf> {
        let existing: Vec<PathBuf> = existing.iter().map(PathBuf::from).collect();
        find_resolve_mcp_with(LocateOptions {
            os,
            program_files: Some(r"D:\Programs".into()),
            exists: &|file| existing.iter().any(|known| known == file),
            install_dir: &|| install_dir.map(PathBuf::from),
        })
    }

    #[test]
    fn finds_resolve_mcp_in_program_files_then_where_the_drp_association_points() {
        let usual = r"D:\Programs\Blackmagic Design\DaVinci Resolve\ResolveMCP.exe";
        assert_eq!(find("windows", &[usual], None), Some(usual.into()));
        let elsewhere = r"E:\Resolve\ResolveMCP.exe";
        assert_eq!(
            find("windows", &[elsewhere], Some(r"E:\Resolve")),
            Some(elsewhere.into())
        );
        assert_eq!(find("windows", &[], Some(r"E:\Resolve")), None);
        assert_eq!(
            win_dirname(r"C:\Program Files\Blackmagic Design\DaVinci Resolve\Resolve.exe"),
            PathBuf::from(r"C:\Program Files\Blackmagic Design\DaVinci Resolve")
        );
    }

    #[test]
    fn finds_resolve_mcp_on_macos_and_linux() {
        assert_eq!(
            find("macos", &[MAC_PATHS[1]], None),
            Some(MAC_PATHS[1].into())
        );
        assert_eq!(find("linux", &[LINUX_PATH], None), Some(LINUX_PATH.into()));
        assert_eq!(find("linux", &[], None), None);
    }
}
