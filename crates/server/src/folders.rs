// Lists folders on this computer for the add-project dialog's folder picker, the way T3 Code's
// "Add project" browses: the page asks for one folder at a time and filters what you type itself.

use std::io::ErrorKind;
use std::path::{MAIN_SEPARATOR_STR, Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::bail;
use linker_edit_protocol::{FolderEntry, FolderListing};

use crate::js;
use crate::projects::store::PROJECT_FILE;

/// More than anyone scrolls through; a folder of camera cards shouldn't send thousands of rows.
const MAX_ENTRIES: usize = 1000;
const ROOTS_TTL: Duration = Duration::from_secs(30);
/// A mapped network drive that's offline can take long to answer; it's left out instead.
const DRIVE_TIMEOUT: Duration = Duration::from_millis(1500);
// Folders Windows keeps at the root of every drive, of no use for a project.
const WINDOWS_SYSTEM: [&str; 5] = [
    "$recycle.bin",
    "system volume information",
    "$windows.~ws",
    "$windows.~bt",
    "config.msi",
];

/// ~ and ~/… as the home folder, like a shell; anything else as given.
pub fn expand_home(value: &str, home: &Path) -> String {
    let trimmed = js::trim(value);
    if trimmed == "~" {
        return home.to_string_lossy().into_owned();
    }
    if let Some(rest) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
    {
        return js::join(home, rest).to_string_lossy().into_owned();
    }
    trimmed.to_string()
}

fn with_separator(dir: &str) -> String {
    if dir.ends_with(MAIN_SEPARATOR_STR) {
        dir.to_string()
    } else {
        format!("{dir}{MAIN_SEPARATOR_STR}")
    }
}

async fn is_project(dir: &Path) -> bool {
    tokio::fs::metadata(dir.join(PROJECT_FILE))
        .await
        .is_ok_and(|metadata| metadata.is_file())
}

async fn is_folder(entry: &tokio::fs::DirEntry) -> bool {
    let Ok(kind) = entry.file_type().await else {
        return false;
    };
    if kind.is_dir() {
        return true;
    }
    // On Windows these are mostly the old junctions such as "Application Data", which can't be opened.
    if !kind.is_symlink() || cfg!(windows) {
        return false;
    }
    tokio::fs::metadata(entry.path())
        .await
        .is_ok_and(|metadata| metadata.is_dir())
}

static ROOTS: Mutex<Option<(Instant, Vec<String>)>> = Mutex::new(None);

async fn drive_exists(drive: String) -> bool {
    let check = tokio::fs::metadata(drive);
    tokio::time::timeout(DRIVE_TIMEOUT, check)
        .await
        .is_ok_and(|found| found.is_ok())
}

/// The drives on Windows, checked at most every half minute; / elsewhere.
pub async fn list_roots() -> Vec<String> {
    if !cfg!(windows) {
        return vec!["/".into()];
    }
    if let Some((at, list)) = ROOTS.lock().unwrap().as_ref()
        && at.elapsed() < ROOTS_TTL
    {
        return list.clone();
    }
    let drives: Vec<String> = ('A'..='Z').map(|letter| format!("{letter}:\\")).collect();
    let checks: Vec<_> = drives
        .iter()
        .map(|drive| tokio::spawn(drive_exists(drive.clone())))
        .collect();
    let mut list = Vec::new();
    for (drive, check) in drives.into_iter().zip(checks) {
        if check.await.unwrap_or(false) {
            list.push(drive);
        }
    }
    *ROOTS.lock().unwrap() = Some((Instant::now(), list.clone()));
    list
}

/// The folders inside dir, an absolute path or one starting with ~; an empty dir is the home folder.
/// Relative paths are refused: the server's working folder means nothing to the person picking.
pub async fn browse_folders(dir: &str, home: &Path) -> anyhow::Result<FolderListing> {
    let expanded = if js::trim(dir).is_empty() {
        home.to_string_lossy().into_owned()
    } else {
        expand_home(dir, home)
    };
    if !js::is_absolute(&expanded) {
        bail!(
            "Start the path from a drive or ~, such as {}.",
            js::join(home, "Videos").display()
        );
    }
    let resolved = with_separator(&js::resolve(Path::new(&expanded)).to_string_lossy());
    let without_separator = PathBuf::from(&resolved[..resolved.len() - 1]);
    let mut entries = Vec::new();
    match tokio::fs::read_dir(&resolved).await {
        Ok(mut dirents) => {
            while let Some(entry) = dirents.next_entry().await? {
                entries.push(entry);
            }
        }
        Err(error) => match error.kind() {
            ErrorKind::PermissionDenied => {}
            // Windows says NotFound for a file with a separator after it.
            kind if kind == ErrorKind::NotADirectory
                || std::fs::metadata(&without_separator)
                    .is_ok_and(|metadata| metadata.is_file()) =>
            {
                bail!("{} is a file, not a folder.", without_separator.display())
            }
            ErrorKind::NotFound => bail!("There's no folder {resolved}"),
            _ => return Err(error.into()),
        },
    }
    let windows = cfg!(windows);
    let mut folders = Vec::new();
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if windows && WINDOWS_SYSTEM.contains(&name.to_lowercase().as_str()) {
            continue;
        }
        if is_folder(&entry).await {
            folders.push(name);
        }
    }
    folders.sort_by(|a, b| js::locale_compare_numeric(a, b));
    folders.truncate(MAX_ENTRIES);
    let mut listed = Vec::new();
    for name in folders {
        let is_project = is_project(&Path::new(&resolved).join(&name)).await;
        listed.push(FolderEntry { name, is_project });
    }
    Ok(FolderListing {
        is_project: is_project(Path::new(&resolved)).await,
        dir: resolved,
        entries: listed,
        roots: list_roots().await,
        home: with_separator(&home.to_string_lossy()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        for dir in ["Promo", "b-roll", ".git", "Clip 10", "Clip 9"] {
            std::fs::create_dir(temp.path().join(dir)).unwrap();
        }
        std::fs::write(temp.path().join("Promo").join("linker-edit.json"), "{}").unwrap();
        std::fs::write(temp.path().join("notes.txt"), "").unwrap();
        std::fs::write(temp.path().join("linker-edit.json"), "{}").unwrap();
        temp
    }

    fn text(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    #[tokio::test]
    async fn lists_only_folders_sorted_the_way_people_count_marking_project_folders() {
        let temp = root();
        let root = temp.path();
        let listing = browse_folders(&text(root), Path::new("/unused"))
            .await
            .unwrap();
        assert_eq!(listing.dir, format!("{}{MAIN_SEPARATOR_STR}", text(root)));
        assert!(listing.is_project);
        // Dot-folders are sent too: the picker shows them once a dot is typed.
        let entries: Vec<(&str, bool)> = listing
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.is_project))
            .collect();
        assert_eq!(
            entries,
            [
                (".git", false),
                ("b-roll", false),
                ("Clip 9", false),
                ("Clip 10", false),
                ("Promo", true)
            ]
        );
        assert!(!listing.roots.is_empty());
    }

    #[tokio::test]
    async fn starts_in_the_home_folder_for_an_empty_path_and_expands_tilde() {
        let temp = root();
        let root = temp.path();
        let with_sep = |path: &Path| format!("{}{MAIN_SEPARATOR_STR}", text(path));
        assert_eq!(browse_folders("", root).await.unwrap().dir, with_sep(root));
        assert_eq!(
            browse_folders(&format!("~{MAIN_SEPARATOR_STR}Promo"), root)
                .await
                .unwrap()
                .dir,
            with_sep(&root.join("Promo"))
        );
        assert_eq!(
            browse_folders("~", root).await.unwrap().home,
            with_sep(root)
        );
    }

    #[tokio::test]
    async fn refuses_relative_paths_and_says_when_a_folder_is_missing_or_a_file() {
        let temp = root();
        let root = temp.path();
        let error = |result: anyhow::Result<FolderListing>| result.unwrap_err().to_string();
        assert!(error(browse_folders("Promo", root).await).starts_with("Start the path"));
        assert!(
            error(browse_folders(&text(&root.join("missing")), root).await)
                .starts_with("There's no folder")
        );
        assert!(
            error(browse_folders(&text(&root.join("notes.txt")), root).await)
                .ends_with("is a file, not a folder.")
        );
    }

    #[test]
    fn expands_only_a_leading_tilde() {
        let home = Path::new("/home/me");
        assert_eq!(expand_home("~", home), "/home/me");
        assert_eq!(
            expand_home(" ~/Videos ", home),
            text(&js::join(home, "Videos"))
        );
        assert_eq!(expand_home("D:/~media", home), "D:/~media");
    }

    #[tokio::test]
    async fn list_roots_includes_the_drive_or_root_this_folder_is_on() {
        let temp = tempfile::tempdir().unwrap();
        let own = temp
            .path()
            .ancestors()
            .last()
            .map(|root| text(root).to_uppercase())
            .unwrap();
        let roots: Vec<String> = list_roots()
            .await
            .iter()
            .map(|root| root.to_uppercase())
            .collect();
        assert!(roots.contains(&own), "{roots:?} has no {own}");
    }
}
