//! ffmpeg as a process: finding it, starting it with its output on a pipe, stopping it.
//!
//! The app ships ffmpeg next to its binary (the server uses it too). From a checkout it is on
//! PATH or in the repo's `node_modules/ffmpeg-static`, as the TypeScript server finds it.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::OnceLock;

use crate::state::ServerHttp;

const NAME: &str = if cfg!(windows) {
    "ffmpeg.exe"
} else {
    "ffmpeg"
};

/// The ffmpeg to use: next to the app, on PATH, or in a checkout's node_modules/ffmpeg-static.
pub fn path() -> Option<&'static Path> {
    static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
    PATH.get_or_init(find).as_deref()
}

fn find() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok();
    let beside = exe.as_ref().map(|exe| exe.with_file_name(NAME));
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join(NAME));
    let starts = [
        exe.and_then(|exe| exe.parent().map(Path::to_path_buf)),
        std::env::current_dir().ok(),
    ];
    let checkout = starts.into_iter().flatten().flat_map(|start| {
        start
            .ancestors()
            .map(|dir| dir.join("node_modules").join("ffmpeg-static").join(NAME))
            .collect::<Vec<_>>()
    });
    beside
        .into_iter()
        .chain(on_path)
        .chain(checkout)
        .find(|candidate| candidate.is_file())
}

/// A running ffmpeg, killed when dropped.
pub struct Process(Child);

impl Process {
    /// None while it runs; then whether it did its job (an error: it couldn't open or decode
    /// the input).
    pub fn exited(&mut self) -> Option<bool> {
        match self.0.try_wait() {
            Ok(Some(status)) => Some(status.success()),
            Ok(None) => None,
            Err(_) => Some(false),
        }
    }

    #[cfg(test)]
    pub fn wait(&mut self) {
        let _ = self.0.wait();
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The arguments that read `url` from the server: its session as a header, before the `-i`.
pub fn input_args(server: &ServerHttp, url: &str, seek: Option<f64>) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    if let Some(bearer) = &server.bearer {
        args.extend([
            "-headers".into(),
            format!("Authorization: Bearer {bearer}\r\n"),
        ]);
    }
    if let Some(seek) = seek.filter(|seek| *seek > 0.) {
        args.extend(["-ss".into(), format!("{seek:.4}")]);
    }
    args.extend(["-i".into(), url.into()]);
    args
}

/// Starts ffmpeg with `args`, its output on a pipe.
pub fn spawn(args: &[String]) -> std::io::Result<(Process, ChildStdout)> {
    let ffmpeg = path().ok_or_else(|| std::io::Error::other("ffmpeg isn't installed"))?;
    let mut command = Command::new(ffmpeg);
    command
        .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    no_window(&mut command);
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().expect("stdout is piped");
    Ok((Process(child), stdout))
}

/// How long the media at `url` runs, from what ffmpeg says about it ("Duration: 00:01:02.50").
pub fn duration(server: &ServerHttp, url: &str) -> Option<f64> {
    let ffmpeg = path()?;
    let mut command = Command::new(ffmpeg);
    command
        .args(["-hide_banner", "-nostdin"])
        .args(input_args(server, url, None))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    no_window(&mut command);
    let mut child = command.spawn().ok()?;
    let mut said = String::new();
    child.stderr.take()?.read_to_string(&mut said).ok()?;
    let _ = child.wait();
    parse_duration(&said)
}

fn parse_duration(said: &str) -> Option<f64> {
    let rest = &said[said.find("Duration: ")? + "Duration: ".len()..];
    let clock = rest.split(',').next()?.trim();
    let mut parts = clock.split(':').map(|part| part.parse::<f64>().ok());
    let (h, m, s) = (parts.next()??, parts.next()??, parts.next()??);
    Some(h * 3600. + m * 60. + s)
}

/// Windows: no console window flashing up for each ffmpeg (the TS's `windowsHide: true`).
fn no_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_duration_ffmpeg_reports() {
        let said = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'a.mp4':\n  Duration: 00:01:02.50, start: 0.000000, bitrate: 350 kb/s\n";
        assert_eq!(parse_duration(said), Some(62.5));
        assert_eq!(parse_duration("  Duration: N/A, bitrate: N/A"), None);
        assert_eq!(
            parse_duration("http://x: Server returned 404 Not Found"),
            None
        );
    }

    #[test]
    fn sends_the_session_before_the_input() {
        let server = ServerHttp {
            base: "https://edit.example".into(),
            bearer: Some("s3ss10n".into()),
        };
        assert_eq!(
            input_args(&server, "https://edit.example/media/a", Some(1.5)),
            [
                "-headers",
                "Authorization: Bearer s3ss10n\r\n",
                "-ss",
                "1.5000",
                "-i",
                "https://edit.example/media/a"
            ]
        );
    }
}
