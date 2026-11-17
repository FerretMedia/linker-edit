// What the TypeScript server read from files next to it (APP_ROOT in config.ts): the Python the Resolve tools run
// and the built browser UI. The Rust server carries them inside its binary instead, so it runs from anywhere and
// nothing depends on where it was installed. package.json's version is updates::VERSION.

/// The Python script a Resolve tool runs, by its file name in apps/desktop/server/resolve/scripts, such as
/// "list_projects.py". None until the Resolve port embeds the scripts here with include_str!.
pub fn resolve_script(name: &str) -> Option<&'static str> {
    let _ = name;
    None
}

/// A file of the built Svelte UI (apps/desktop/build) by its path there, such as "index.html" or
/// "_app/immutable/entry/start.js". None until the HTTP port embeds the build here.
pub fn web_file(path: &str) -> Option<&'static [u8]> {
    let _ = path;
    None
}
