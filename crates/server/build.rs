// A release build carries the built Svelte UI (assets::web_file): build again when `npm run build` changed it. A
// path that doesn't exist would make Cargo run this every time, so a missing build is only noticed by a clean
// build; a debug build reads the folder at run time anyway.
fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if std::path::Path::new("../../apps/desktop/build").is_dir() {
        println!("cargo::rerun-if-changed=../../apps/desktop/build");
    }
}
