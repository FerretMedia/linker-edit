// Finding the `claude` and `codex` programs. The TypeScript got them from the npm SDKs, which bundle them;
// the Rust app ships them next to its own binary, and a build from source uses the ones on PATH. Neither SDK
// option for another path (pathToClaudeCodeExecutable, codexPathOverride) was used, so there is no override.
//
// The layout packaging ships, beside the linker-edit binary:
//
//   claude(.exe)                  Claude Code's native binary, as @anthropic-ai/claude-agent-sdk-<platform> has it
//   codex/                        @openai/codex-<platform>'s vendor/<target triple> folder, unchanged:
//     bin/codex(.exe)               the program
//     codex-path/                   put first on its PATH (rg), as the Codex SDK does
//     codex-resources/              its helpers (the Windows sandbox), which it finds itself
//     codex-package.json

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The Claude Code version @anthropic-ai/claude-agent-sdk 0.3.316 bundled (its manifest.json). Packaging
/// ships this one, so the CLI behind the stream-json protocol is the one the TypeScript was tested with.
pub const CLAUDE_CODE_VERSION: &str = "2.1.316";
/// The Agent SDK whose `query()` claude_cli.rs replaces; Claude Code is told this version, as the SDK told it.
pub const AGENT_SDK_VERSION: &str = "0.3.316";
/// The codex version @openai/codex-sdk 0.166.0 depends on (@openai/codex 0.166.0). Packaging ships this one.
pub const CODEX_VERSION: &str = "0.166.0";

const EXE: &str = std::env::consts::EXE_SUFFIX;

/// A program to run, with folders to put first on its PATH.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliProgram {
    pub program: PathBuf,
    pub path_dirs: Vec<PathBuf>,
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(Path::to_path_buf)
}

fn on_path(name: &str, path: Option<&OsStr>) -> Option<PathBuf> {
    // Native programs only: npm's .cmd shims would need cmd.exe and its quoting for our JSON arguments.
    let file = format!("{name}{EXE}");
    std::env::split_paths(path?)
        .map(|dir| dir.join(&file))
        .find(|candidate| candidate.is_file())
}

/// Claude Code: bundled beside the app, then PATH.
pub fn find_claude() -> Option<PathBuf> {
    find_claude_in(exe_dir().as_deref(), std::env::var_os("PATH").as_deref())
}

pub fn find_claude_in(exe_dir: Option<&Path>, path: Option<&OsStr>) -> Option<PathBuf> {
    exe_dir
        .map(|dir| dir.join(format!("claude{EXE}")))
        .filter(|bundled| bundled.is_file())
        .or_else(|| on_path("claude", path))
}

/// Codex: bundled beside the app (with its codex-path folder), then PATH.
pub fn find_codex() -> Option<CliProgram> {
    find_codex_in(exe_dir().as_deref(), std::env::var_os("PATH").as_deref())
}

pub fn find_codex_in(exe_dir: Option<&Path>, path: Option<&OsStr>) -> Option<CliProgram> {
    if let Some(root) = exe_dir.map(|dir| dir.join("codex")) {
        let program = root.join("bin").join(format!("codex{EXE}"));
        if program.is_file() {
            let path_dir = root.join("codex-path");
            return Some(CliProgram {
                program,
                path_dirs: if path_dir.is_dir() {
                    vec![path_dir]
                } else {
                    vec![]
                },
            });
        }
    }
    on_path("codex", path).map(|program| CliProgram {
        program,
        path_dirs: vec![],
    })
}

/// The Codex SDK's prependPathDirs: the folders first on PATH, once each. On Windows the variable keeps the
/// name it had ("Path" usually), and other spellings of it go.
pub fn prepend_path_dirs(env: &mut Vec<(String, String)>, dirs: &[PathBuf]) {
    if dirs.is_empty() {
        return;
    }
    let key = if cfg!(windows) {
        let keys: Vec<&String> = env
            .iter()
            .map(|(key, _)| key)
            .filter(|key| key.eq_ignore_ascii_case("path"))
            .collect();
        if keys.iter().any(|key| *key == "Path") {
            "Path".to_string()
        } else {
            keys.last()
                .map(|key| key.to_string())
                .unwrap_or_else(|| "PATH".into())
        }
    } else {
        "PATH".to_string()
    };
    let existing = env
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, value)| value.clone())
        .unwrap_or_default();
    if cfg!(windows) {
        env.retain(|(name, _)| !name.eq_ignore_ascii_case("path") || *name == key);
    }
    let dirs: Vec<String> = dirs.iter().map(|dir| dir.display().to_string()).collect();
    let mut entries = dirs.clone();
    entries.extend(
        std::env::split_paths(&existing)
            .map(|entry| entry.display().to_string())
            .filter(|entry| !entry.is_empty() && !dirs.contains(entry)),
    );
    let joined = std::env::join_paths(&entries)
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|_| entries.join(if cfg!(windows) { ";" } else { ":" }));
    match env.iter_mut().find(|(name, _)| *name == key) {
        Some(entry) => entry.1 = joined,
        None => env.push((key, joined)),
    }
}

/// tokio's Command for an agent's program: no console window on Windows (the TypeScript's windowsHide).
pub fn command(program: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    crate::js::hide_window(&mut command);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(file: &Path) {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, b"").unwrap();
    }

    #[test]
    fn finds_the_bundled_programs_before_the_ones_on_path() {
        let app = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let path = std::env::join_paths([elsewhere.path()]).unwrap();
        touch(&elsewhere.path().join(format!("claude{EXE}")));
        touch(&elsewhere.path().join(format!("codex{EXE}")));

        assert_eq!(
            find_claude_in(Some(app.path()), Some(&path)),
            Some(elsewhere.path().join(format!("claude{EXE}")))
        );
        assert_eq!(
            find_codex_in(Some(app.path()), Some(&path)),
            Some(CliProgram {
                program: elsewhere.path().join(format!("codex{EXE}")),
                path_dirs: vec![]
            })
        );

        touch(&app.path().join(format!("claude{EXE}")));
        touch(
            &app.path()
                .join("codex")
                .join("bin")
                .join(format!("codex{EXE}")),
        );
        std::fs::create_dir_all(app.path().join("codex").join("codex-path")).unwrap();
        assert_eq!(
            find_claude_in(Some(app.path()), Some(&path)),
            Some(app.path().join(format!("claude{EXE}")))
        );
        assert_eq!(
            find_codex_in(Some(app.path()), Some(&path)),
            Some(CliProgram {
                program: app
                    .path()
                    .join("codex")
                    .join("bin")
                    .join(format!("codex{EXE}")),
                path_dirs: vec![app.path().join("codex").join("codex-path")]
            })
        );
        assert_eq!(find_claude_in(None, None), None);
    }

    #[test]
    fn puts_codexs_own_folders_first_on_its_path_once() {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let key = if cfg!(windows) { "Path" } else { "PATH" };
        let mut env = vec![
            (key.to_string(), format!("/usr/bin{sep}/tools")),
            ("HOME".to_string(), "/home/you".to_string()),
        ];
        prepend_path_dirs(&mut env, &[PathBuf::from("/tools")]);
        assert_eq!(
            env,
            vec![
                (key.to_string(), format!("/tools{sep}/usr/bin")),
                ("HOME".to_string(), "/home/you".to_string()),
            ]
        );
    }
}
