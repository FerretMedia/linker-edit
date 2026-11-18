// What the TypeScript server read from files next to it (APP_ROOT in config.ts): the Python the Resolve tools run
// and the built browser UI. The Rust server carries them inside its binary instead, so it runs from anywhere and
// nothing depends on where it was installed. package.json's version is updates::VERSION.

/// The Python script a Resolve tool runs, by its file name in crates/server/src/resolve/scripts, such as
/// "list_projects.py". None for a name that isn't one of them.
pub fn resolve_script(name: &str) -> Option<&'static str> {
    RESOLVE_SCRIPTS
        .iter()
        .find(|(file, _)| *file == name)
        .map(|(_, text)| *text)
}

/// Every Resolve script, by file name. A script added to the folder goes here too; resolve::scripts' tests check.
pub const RESOLVE_SCRIPTS: &[(&str, &str)] = &[
    (
        "apply_timeline.py",
        include_str!("resolve/scripts/apply_timeline.py"),
    ),
    (
        "backup_project.py",
        include_str!("resolve/scripts/backup_project.py"),
    ),
    ("common.py", include_str!("resolve/scripts/common.py")),
    (
        "list_projects.py",
        include_str!("resolve/scripts/list_projects.py"),
    ),
    (
        "open_project.py",
        include_str!("resolve/scripts/open_project.py"),
    ),
    (
        "projects_common.py",
        include_str!("resolve/scripts/projects_common.py"),
    ),
    (
        "render_finish.py",
        include_str!("resolve/scripts/render_finish.py"),
    ),
    (
        "render_page.py",
        include_str!("resolve/scripts/render_page.py"),
    ),
    (
        "render_poll.py",
        include_str!("resolve/scripts/render_poll.py"),
    ),
    (
        "render_start.py",
        include_str!("resolve/scripts/render_start.py"),
    ),
    (
        "render_stop.py",
        include_str!("resolve/scripts/render_stop.py"),
    ),
    (
        "scene_cuts_start.py",
        include_str!("resolve/scripts/scene_cuts_start.py"),
    ),
    (
        "scene_cuts_status.py",
        include_str!("resolve/scripts/scene_cuts_status.py"),
    ),
    ("snapshot.py", include_str!("resolve/scripts/snapshot.py")),
    ("status.py", include_str!("resolve/scripts/status.py")),
    (
        "timeline_stills.py",
        include_str!("resolve/scripts/timeline_stills.py"),
    ),
    (
        "tool_add_clips.py",
        include_str!("resolve/scripts/tool_add_clips.py"),
    ),
    (
        "tool_add_markers.py",
        include_str!("resolve/scripts/tool_add_markers.py"),
    ),
    (
        "tool_add_title.py",
        include_str!("resolve/scripts/tool_add_title.py"),
    ),
    (
        "tool_create_timeline.py",
        include_str!("resolve/scripts/tool_create_timeline.py"),
    ),
    (
        "tool_duplicate_timeline.py",
        include_str!("resolve/scripts/tool_duplicate_timeline.py"),
    ),
    (
        "tool_export_timeline.py",
        include_str!("resolve/scripts/tool_export_timeline.py"),
    ),
    (
        "tool_get_clip_properties.py",
        include_str!("resolve/scripts/tool_get_clip_properties.py"),
    ),
    (
        "tool_get_timeline.py",
        include_str!("resolve/scripts/tool_get_timeline.py"),
    ),
    (
        "tool_get_transcription.py",
        include_str!("resolve/scripts/tool_get_transcription.py"),
    ),
    (
        "tool_import_media.py",
        include_str!("resolve/scripts/tool_import_media.py"),
    ),
    (
        "tool_import_timeline.py",
        include_str!("resolve/scripts/tool_import_timeline.py"),
    ),
    (
        "tool_list_media.py",
        include_str!("resolve/scripts/tool_list_media.py"),
    ),
    (
        "tool_list_timelines.py",
        include_str!("resolve/scripts/tool_list_timelines.py"),
    ),
    (
        "tool_organize_media.py",
        include_str!("resolve/scripts/tool_organize_media.py"),
    ),
    (
        "tool_remove_clips.py",
        include_str!("resolve/scripts/tool_remove_clips.py"),
    ),
    (
        "tool_remove_markers.py",
        include_str!("resolve/scripts/tool_remove_markers.py"),
    ),
    (
        "tool_render_options.py",
        include_str!("resolve/scripts/tool_render_options.py"),
    ),
    (
        "tool_resolve_context.py",
        include_str!("resolve/scripts/tool_resolve_context.py"),
    ),
    (
        "tool_set_clip_properties.py",
        include_str!("resolve/scripts/tool_set_clip_properties.py"),
    ),
    (
        "tool_set_color.py",
        include_str!("resolve/scripts/tool_set_color.py"),
    ),
    (
        "tool_set_current_timeline.py",
        include_str!("resolve/scripts/tool_set_current_timeline.py"),
    ),
    (
        "tool_set_tracks.py",
        include_str!("resolve/scripts/tool_set_tracks.py"),
    ),
    (
        "tool_transcribe_in_resolve.py",
        include_str!("resolve/scripts/tool_transcribe_in_resolve.py"),
    ),
];

/// A file of the built Svelte UI (apps/desktop/build) by its path there, such as "index.html" or
/// "_app/immutable/entry/start.js". None until the HTTP port embeds the build here.
pub fn web_file(path: &str) -> Option<&'static [u8]> {
    let _ = path;
    None
}
