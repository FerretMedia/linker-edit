//! src/lib/library.ts: the library page's tabs, and how its files are shown and played.
//!
//! The browser keeps the tab in the address ("#library/project"); the window has no address, so
//! `libraryTabFromHash`/`libraryHash` have no port: the workspace holds the open tab.

use linker_edit_protocol::{LibraryKind, LibraryScope};

/// The library page's tabs. Workspaces and series will add theirs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LibraryTab {
    #[default]
    Universal,
    Project,
}

pub fn scope_of(tab: LibraryTab, project_id: Option<&str>) -> Option<LibraryScope> {
    match tab {
        LibraryTab::Universal => Some(LibraryScope::Universal),
        LibraryTab::Project => project_id.map(|project_id| LibraryScope::Project {
            project_id: project_id.into(),
        }),
    }
}

pub fn kind_label(kind: LibraryKind) -> &'static str {
    match kind {
        LibraryKind::Music => "Music",
        LibraryKind::Sfx => "Sound effects",
        LibraryKind::Graphics => "Graphics",
        LibraryKind::Luts => "LUTs",
        LibraryKind::Footage => "Footage",
        LibraryKind::Other => "Other",
    }
}

/// What plays (with an audio element in the browser), as the server's library route serves them.
const PLAYS: &[&str] = &[
    ".mp3", ".wav", ".m4a", ".aac", ".ogg", ".opus", ".flac", ".aif", ".aiff", ".mp4", ".m4v",
    ".mov", ".webm",
];
/// What shows as a picture.
const SHOWS: &[&str] = &[".png", ".jpg", ".jpeg", ".gif", ".webp"];

fn extension(name: &str) -> String {
    name.rfind('.')
        .filter(|&dot| dot + 1 < name.len())
        .map(|dot| name[dot..].to_lowercase())
        .unwrap_or_default()
}

pub fn playable(name: &str) -> bool {
    PLAYS.contains(&extension(name).as_str())
}

pub fn showable(name: &str) -> bool {
    SHOWS.contains(&extension(name).as_str())
}

/// JavaScript's encodeURIComponent: everything but `A-Z a-z 0-9 - _ . ! ~ * ' ( )`, as UTF-8.
pub fn encode_uri_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Where a library file plays from on the server, as a path to put after its address.
pub fn file_url(folder_id: &str, rel: &str) -> String {
    let rel = rel
        .split('/')
        .map(encode_uri_component)
        .collect::<Vec<_>>()
        .join("/");
    format!("/library/{}/{rel}", encode_uri_component(folder_id))
}

/// 3:07, or 1:02:03 for an hour or more; under 10 seconds with a tenth, like 1.2s.
pub fn format_duration(seconds: f64) -> String {
    if seconds < 10. {
        return format!("{}s", (seconds * 10.).round() / 10.);
    }
    let total = seconds.round() as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Tags as typed in one field, split on commas.
pub fn parse_tags(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // library.test.ts
    #[test]
    fn writes_lengths_the_way_editors_read_them() {
        assert_eq!(format_duration(1.23), "1.2s");
        assert_eq!(format_duration(187.4), "3:07");
        assert_eq!(format_duration(3723.), "1:02:03");
    }

    #[test]
    fn encodes_each_part_of_a_files_path() {
        assert_eq!(
            file_url("id", "Calm/Piano #1.wav"),
            "/library/id/Calm/Piano%20%231.wav"
        );
    }

    #[test]
    fn splits_tags_on_commas() {
        assert_eq!(parse_tags(" happy, intro ,, "), ["happy", "intro"]);
    }

    #[test]
    fn plays_sound_and_shows_pictures() {
        assert!(playable("Intro.MP3"));
        assert!(showable("logo.webp"));
        assert!(!playable("notes.txt") && !showable("README"));
    }
}
