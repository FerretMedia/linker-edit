//! On Windows the release exe is a GUI program, so opening the app from the Start menu shows no console window.
//! A GUI program started from a terminal isn't given that terminal's console, so `serve`, `--browser` and
//! `--version` attach to it to print there. Started with pipes instead (an agent running `linker-edit mcp`, the
//! npm launcher), it keeps them. cmd and PowerShell don't wait for a GUI program, so their prompt comes back while
//! `serve` runs; Ctrl+C still stops it.

#[cfg(windows)]
pub fn attach() {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE,
    };
    // SAFETY: plain Win32 calls with constant arguments; the handle is only compared.
    unsafe {
        let out = GetStdHandle(STD_OUTPUT_HANDLE);
        if !out.is_null() && out != INVALID_HANDLE_VALUE {
            return;
        }
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
pub fn attach() {}
