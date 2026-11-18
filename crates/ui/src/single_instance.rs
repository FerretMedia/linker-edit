//! One window per data folder, like Electron's requestSingleInstanceLock in main.ts: a second
//! launch brings the first one's window forward and quits, starting no server of its own.
//!
//! The first copy listens on a local socket (a named pipe on Windows) named after the data
//! folder, so a copy with its own `LINKER_EDIT_CONFIG` (a test, another agent) runs alongside.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, prelude::*};

pub enum Instance {
    /// This is the first copy; a message arrives each time another one starts.
    Primary(UnboundedReceiver<()>),
    /// Another copy runs and was asked to show its window; quit.
    Secondary,
}

/// Claims the lock, or hands over to the copy that holds it.
pub fn claim() -> anyhow::Result<Instance> {
    let name = socket_name();
    if let Ok(mut stream) = Stream::connect(name.as_str().to_ns_name::<GenericNamespaced>()?) {
        allow_foreground();
        stream.write_all(b"focus\n")?;
        return Ok(Instance::Secondary);
    }
    let listener = ListenerOptions::new()
        .name(name.as_str().to_ns_name::<GenericNamespaced>()?)
        // A copy that crashed may leave its socket file behind on macOS.
        .try_overwrite(true)
        .create_sync()?;
    let (tx, rx) = unbounded();
    std::thread::Builder::new()
        .name("single-instance".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut line = String::new();
                if BufReader::new(stream).read_line(&mut line).is_ok()
                    && line.trim() == "focus"
                    && tx.unbounded_send(()).is_err()
                {
                    break;
                }
            }
        })?;
    Ok(Instance::Primary(rx))
}

/// `linker-edit-<hash of the data folder>.sock`. crates/app checks it before starting a server.
pub fn socket_name() -> String {
    let folder = std::env::var_os("LINKER_EDIT_CONFIG")
        .map(PathBuf::from)
        .or_else(|| std::env::home_dir().map(|home| home.join("LinkerEdit")))
        .unwrap_or_default();
    format!(
        "linker-edit-{:016x}.sock",
        fnv1a(folder.to_string_lossy().as_bytes())
    )
}

/// Stable across Rust versions, unlike `DefaultHasher`, so two builds agree on the name.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Windows only lets the app the user just started take the foreground; this copy is that
/// app, so it passes the right on to the first copy before asking it to come forward.
fn allow_foreground() {
    #[cfg(windows)]
    // SAFETY: a plain Win32 call with a constant argument.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(
            windows_sys::Win32::UI::WindowsAndMessaging::ASFW_ANY,
        );
    }
}
