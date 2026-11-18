// What the `linker-edit` command does besides running the server: its version, and opening the browser. The port
// of server/cli.ts; the binary (crates/app) reads the arguments.

use linker_edit_protocol::release_name::release_name;

use crate::updates::VERSION;

/// `linker-edit --version`: the release name, such as 2026.11.07.
pub fn version_text() -> String {
    release_name(VERSION)
}

/// Opens url in the default browser, or says to when that can't be done.
pub fn open_browser(url: &str) {
    let mut command = if cfg!(windows) {
        let mut command = std::process::Command::new("cmd");
        command.args(["/c", "start", "", url]);
        command
    } else {
        let mut command = std::process::Command::new(if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        });
        command.arg(url);
        command
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    match command.spawn() {
        // Reaped in the background, so it doesn't linger as a zombie on Unix.
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(_) => println!("Open {url} in your browser."),
    }
}
