// On Windows, the exe carries the app's icon and its name, which Explorer, the taskbar and the installer show.
// The version in its details is the crate's, stamped at release (CARGO_PKG_VERSION).
fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=icons/icon.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("icons/icon.ico")
            .set("ProductName", "Linker-Edit")
            .set("FileDescription", "Linker-Edit")
            .set(
                "LegalCopyright",
                "Copyright © 2026 Linker-Edit contributors",
            );
        resource
            .compile()
            .expect("the Windows resources (icon, version info) compile");
    }
}
