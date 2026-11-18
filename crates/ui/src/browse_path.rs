//! Path handling for the folder picker: a port of src/lib/browsePath.ts, adapted from T3 Code's
//! project browsing (packages/client-runtime/src/state/projects.ts and filesystem.ts, MIT; see
//! NOTICE). The path is typed as text for the server's computer, which may run another system
//! than this window.

use linker_edit_protocol::FolderEntry;

pub fn is_windows_drive_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes.len() == 2 || bytes[2] == b'/' || bytes[2] == b'\\')
}

pub fn is_unc_path(value: &str) -> bool {
    value.starts_with("\\\\")
}

#[derive(PartialEq)]
enum Kind {
    Unix,
    Windows,
}

fn absolute_kind(value: &str) -> Option<Kind> {
    if is_windows_drive_path(value) || is_unc_path(value) {
        Some(Kind::Windows)
    } else if value.starts_with('/') {
        Some(Kind::Unix)
    } else {
        None
    }
}

fn preferred_separator(value: &str) -> char {
    match absolute_kind(value) {
        Some(Kind::Windows) => '\\',
        Some(Kind::Unix) => '/',
        None if value.contains('\\') => '\\',
        None => '/',
    }
}

pub fn has_trailing_separator(value: &str) -> bool {
    if absolute_kind(value) == Some(Kind::Unix) {
        value.ends_with('/')
    } else {
        value.ends_with(['/', '\\'])
    }
}

fn last_separator_index(value: &str) -> Option<usize> {
    if absolute_kind(value) == Some(Kind::Unix) {
        value.rfind('/')
    } else {
        value.rfind(['/', '\\'])
    }
}

/// The folder part of what's typed: everything up to and including the last separator.
pub fn browse_directory(value: &str) -> String {
    if has_trailing_separator(value) {
        return value.into();
    }
    if value == "~" {
        return format!("~{}", preferred_separator(value));
    }
    match last_separator_index(value) {
        Some(index) => value[..=index].into(),
        None => String::new(),
    }
}

/// What's typed after the last separator, which filters the folder's entries.
pub fn browse_filter(value: &str) -> String {
    if has_trailing_separator(value) || value == "~" {
        return String::new();
    }
    match last_separator_index(value) {
        Some(index) => value[index + 1..].into(),
        None => value.into(),
    }
}

/// Going into a folder: its name and a separator after the folder part.
pub fn append_segment(value: &str, segment: &str) -> String {
    format!(
        "{}{segment}{}",
        browse_directory(value),
        preferred_separator(value)
    )
}

struct Absolute {
    root: String,
    separator: char,
    segments: Vec<String>,
}

fn split(rest: &str, separator: char) -> Vec<String> {
    rest.split(|c| c == separator || (separator == '\\' && c == '/'))
        .filter(|part| !part.is_empty())
        .map(String::from)
        .collect()
}

fn split_absolute(value: &str) -> Option<Absolute> {
    if is_windows_drive_path(value) {
        return Some(Absolute {
            root: format!("{}\\", &value[..2]),
            separator: '\\',
            segments: split(&value[2..], '\\'),
        });
    }
    if is_unc_path(value) {
        let mut parts = split(value, '\\').into_iter();
        let (server, share) = (parts.next()?, parts.next()?);
        return Some(Absolute {
            root: format!("\\\\{server}\\{share}\\"),
            separator: '\\',
            segments: parts.collect(),
        });
    }
    value.strip_prefix('/').map(|rest| Absolute {
        root: "/".into(),
        separator: '/',
        segments: split(rest, '/'),
    })
}

/// The folder above a folder path, ending in a separator; None at a drive or the root.
pub fn parent_directory(value: &str) -> Option<String> {
    let trimmed = trim_separators(value);
    if let Some(absolute) = split_absolute(&trimmed) {
        return match absolute.segments.len() {
            0 => None,
            1 => Some(absolute.root),
            n => Some(format!(
                "{}{}{}",
                absolute.root,
                absolute.segments[..n - 1].join(&absolute.separator.to_string()),
                absolute.separator
            )),
        };
    }
    last_separator_index(&trimmed).map(|index| trimmed[..=index].to_string())
}

/// Without trailing separators, except for a root such as C:\ or /, which needs one.
pub fn trim_separators(value: &str) -> String {
    let trimmed = value.trim();
    let stripped = trimmed.trim_end_matches(['\\', '/']);
    if stripped.is_empty() {
        return trimmed.chars().next().map(String::from).unwrap_or_default();
    }
    let bytes = stripped.as_bytes();
    if bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return format!("{stripped}\\");
    }
    stripped.to_string()
}

/// The entries that start with the filter, ignoring case; dot-folders only once a dot is typed.
pub fn filter_entries<'a>(entries: &'a [FolderEntry], filter: &str) -> Vec<&'a FolderEntry> {
    let lower = filter.to_lowercase();
    let show_hidden = filter.starts_with('.');
    entries
        .iter()
        .filter(|entry| {
            entry.name.to_lowercase().starts_with(&lower)
                && (show_hidden || !entry.name.starts_with('.'))
        })
        .collect()
}

/// What Tab does to the name being typed.
#[derive(Debug, PartialEq, Eq)]
pub struct Completion {
    pub name: String,
    /// One match: go into it.
    pub complete: bool,
}

/// Shell-style Tab: one match goes into it, several extend the filter to what they share.
pub fn tab_completion(visible: &[&FolderEntry], filter: &str) -> Option<Completion> {
    let (first, rest) = visible.split_first()?;
    if rest.is_empty() {
        return Some(Completion {
            name: first.name.clone(),
            complete: true,
        });
    }
    let mut prefix: Vec<char> = first.name.chars().collect();
    for entry in rest {
        let characters: Vec<char> = entry.name.chars().collect();
        let length = prefix
            .iter()
            .zip(&characters)
            .take_while(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
            .count();
        prefix.truncate(length);
    }
    let name: String = prefix.into_iter().collect();
    (name.chars().count() > filter.chars().count()).then_some(Completion {
        name,
        complete: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // From src/lib/browsePath.test.ts.
    fn entries(names: &[&str]) -> Vec<FolderEntry> {
        names
            .iter()
            .map(|name| FolderEntry {
                name: (*name).into(),
                is_project: false,
            })
            .collect()
    }

    /// A Windows path written with forward slashes, to spare the backslash escapes.
    fn win(value: &str) -> String {
        value.replace('/', "\\")
    }

    #[test]
    fn splits_into_the_folder_to_list_and_the_filter_after_it() {
        let split = |value: &str| (browse_directory(value), browse_filter(value));
        assert_eq!(
            split(&win("C:/Users/jar")),
            (win("C:/Users/"), "jar".into())
        );
        assert_eq!(split(&win("C:/Users/")), (win("C:/Users/"), "".into()));
        assert_eq!(split("/home/me/Vid"), ("/home/me/".into(), "Vid".into()));
        assert_eq!(split("~"), ("~/".into(), "".into()));
        assert_eq!(browse_directory("Videos"), "");
    }

    #[test]
    fn goes_into_a_folder_with_the_paths_own_separator() {
        assert_eq!(
            append_segment(&win("C:/Users/ja"), "jarno"),
            win("C:/Users/jarno/")
        );
        assert_eq!(append_segment("/home/", "me"), "/home/me/");
        assert_eq!(append_segment("~/", "Videos"), "~/Videos/");
    }

    #[test]
    fn goes_up_to_a_drive_a_share_or_the_root_and_no_further() {
        assert_eq!(
            parent_directory(&win("C:/Users/jarno/")),
            Some(win("C:/Users/"))
        );
        assert_eq!(parent_directory(&win("C:/Users/")), Some(win("C:/")));
        assert_eq!(parent_directory(&win("C:/")), None);
        assert_eq!(
            parent_directory(&win("//nas/media/Promo/")),
            Some(win("//nas/media/"))
        );
        assert_eq!(parent_directory(&win("//nas/media/")), None);
        assert_eq!(parent_directory("/home/"), Some("/".into()));
        assert_eq!(parent_directory("/"), None);
    }

    #[test]
    fn trims_trailing_separators_but_keeps_a_roots() {
        assert_eq!(
            trim_separators(&win("C:/Users/jarno/")),
            win("C:/Users/jarno")
        );
        assert_eq!(trim_separators(&win("C:/")), win("C:/"));
        assert_eq!(trim_separators("C:"), win("C:/"));
        assert_eq!(trim_separators("/"), "/");
        assert_eq!(trim_separators(" /media/ "), "/media");
        assert_eq!(trim_separators(""), "");
    }

    #[test]
    fn matches_the_start_of_names_ignoring_case_and_hides_dot_folders_until_a_dot() {
        let list = entries(&[".git", "Promo", "promo cuts", "B-roll"]);
        let names = |filter: &str| -> Vec<String> {
            filter_entries(&list, filter)
                .iter()
                .map(|entry| entry.name.clone())
                .collect()
        };
        assert_eq!(names("pro"), ["Promo", "promo cuts"]);
        assert_eq!(names(""), ["Promo", "promo cuts", "B-roll"]);
        assert_eq!(names("."), [".git"]);
    }

    #[test]
    fn completes_one_match_fully_and_several_to_what_they_share() {
        let complete = |names: &[&str], filter: &str| {
            let list = entries(names);
            let visible: Vec<&FolderEntry> = list.iter().collect();
            tab_completion(&visible, filter)
        };
        assert_eq!(
            complete(&["Promo"], "pr"),
            Some(Completion {
                name: "Promo".into(),
                complete: true
            })
        );
        assert_eq!(
            complete(&["Promo final", "Promo draft"], "pr"),
            Some(Completion {
                name: "Promo ".into(),
                complete: false
            })
        );
        assert_eq!(complete(&["Promo", "Prores"], "pro"), None);
        assert_eq!(complete(&[], "x"), None);
    }
}
