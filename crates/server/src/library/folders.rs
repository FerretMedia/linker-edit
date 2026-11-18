// Where the library's folders are written down. The universal library and what the user notes on each file
// (tags, star, note, "used in") are in ~/LinkerEdit/library.json; a project's own folders are in its
// .linker-edit/local.json, because their paths belong to this machine and are never committed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail};
use linker_edit_protocol::{
    LIBRARY_KINDS, LibraryFolder, LibraryFolderFields, LibraryKind, LibraryUse, LocalLink,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::js;

/// What the user keeps on one file, by its absolute path.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileData {
    pub tags: Vec<String>,
    pub star: bool,
    pub note: String,
    pub used_in: Vec<LibraryUse>,
}

impl FileData {
    fn is_empty(&self) -> bool {
        self.tags.is_empty() && !self.star && self.note.is_empty() && self.used_in.is_empty()
    }
}

/// library.json as it is on disk.
#[derive(Debug, Clone, Default, Serialize)]
struct LibraryJson {
    version: u32,
    folders: Vec<LibraryFolder>,
    files: BTreeMap<String, FileData>,
}

// Bump when the file changes shape; a file with another version is kept aside rather than misread.
const VERSION: u32 = 1;
const MAX_TAGS: usize = 30;
const MAX_TEXT: usize = 2000;

/// The kind a JSON value names, if it names one.
pub fn library_kind(value: &Value) -> Option<LibraryKind> {
    let name = value.as_str()?;
    LIBRARY_KINDS.into_iter().find(|kind| {
        serde_json::to_value(kind)
            .ok()
            .as_ref()
            .and_then(Value::as_str)
            == Some(name)
    })
}

/// A folder from a file or a message, cleaned up; None when it isn't one.
pub fn clean_folder(value: &Value) -> Option<LibraryFolder> {
    let raw = value.as_object()?;
    let id = raw.get("id")?.as_str().filter(|id| !id.is_empty())?;
    let kind = library_kind(raw.get("kind")?)?;
    let path = raw
        .get("path")?
        .as_str()
        .map(js::trim)
        .filter(|path| !path.is_empty())?;
    let folder_path = js::resolve(path).to_string_lossy().into_owned();
    let label = match raw.get("label").and_then(Value::as_str).map(js::trim) {
        Some(label) if !label.is_empty() => js::slice(label, 200).to_string(),
        _ => Path::new(&folder_path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| folder_path.clone()),
    };
    let note = raw
        .get("note")
        .and_then(Value::as_str)
        .map(|note| js::slice(js::trim(note), MAX_TEXT).to_string())
        .unwrap_or_default();
    Some(LibraryFolder {
        id: id.to_string(),
        kind,
        label,
        path: folder_path,
        note,
    })
}

/// A new folder from what the user filled in; an error says what's wrong.
pub fn new_folder(
    fields: &LibraryFolderFields,
    taken: &[LibraryFolder],
) -> anyhow::Result<LibraryFolder> {
    if !js::is_absolute(js::trim(&fields.path)) {
        bail!("Pick the folder, starting from a drive or ~.");
    }
    let mut value = serde_json::to_value(fields)?;
    value["id"] = Value::from(uuid::Uuid::new_v4().to_string());
    let folder = clean_folder(&value).ok_or_else(|| anyhow!("Pick a kind and a folder."))?;
    check_folder(&folder, taken)?;
    Ok(folder)
}

/// Refuses a folder that's missing, a file, or already in this library.
pub fn check_folder(folder: &LibraryFolder, others: &[LibraryFolder]) -> anyhow::Result<()> {
    let Ok(stat) = std::fs::metadata(&folder.path) else {
        bail!("There is no folder {} on this computer.", folder.path);
    };
    if !stat.is_dir() {
        bail!("{} is a file, not a folder.", folder.path);
    }
    if let Some(same) = others
        .iter()
        .find(|other| other.id != folder.id && same_path(&other.path, &folder.path))
    {
        bail!(
            "{} is already in this library as \"{}\".",
            folder.path,
            same.label
        );
    }
    Ok(())
}

/// Windows paths are the same whatever their case.
pub fn path_key(file: impl AsRef<Path>) -> String {
    let resolved = js::resolve(file).to_string_lossy().into_owned();
    if cfg!(windows) {
        resolved.to_lowercase()
    } else {
        resolved
    }
}

pub fn same_path(a: impl AsRef<Path>, b: impl AsRef<Path>) -> bool {
    path_key(a) == path_key(b)
}

fn clean_data(value: &Value) -> Option<FileData> {
    let raw = value.as_object()?;
    let used_in = raw
        .get("usedIn")
        .and_then(Value::as_array)
        .map(|uses| uses.iter().filter_map(clean_use).collect())
        .unwrap_or_default();
    Some(FileData {
        tags: clean_tags(raw.get("tags").unwrap_or(&Value::Null)),
        star: raw.get("star") == Some(&Value::Bool(true)),
        note: raw
            .get("note")
            .and_then(Value::as_str)
            .map(|note| js::slice(note, MAX_TEXT).to_string())
            .unwrap_or_default(),
        used_in,
    })
}

fn clean_use(value: &Value) -> Option<LibraryUse> {
    let raw = value.as_object()?;
    let text = |key: &str| raw.get(key).and_then(Value::as_str).map(str::to_string);
    Some(LibraryUse {
        project_id: text("projectId")?,
        project: text("project").unwrap_or_default(),
        timeline: text("timeline"),
        at: text("at")?,
    })
}

/// Tags as the user typed them, without duplicates or empty ones.
pub fn clean_tags(value: &Value) -> Vec<String> {
    let Some(values) = value.as_array() else {
        return Vec::new();
    };
    let mut tags: Vec<String> = Vec::new();
    for tag in values.iter().filter_map(Value::as_str) {
        let clean = js::trim(tag)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let clean = js::slice(&clean, 60).to_string();
        if !clean.is_empty()
            && !tags
                .iter()
                .any(|other| other.to_lowercase() == clean.to_lowercase())
        {
            tags.push(clean);
        }
    }
    tags.truncate(MAX_TAGS);
    tags
}

/// Where the universal library's folders and what's noted on files are kept.
pub trait LibraryStore: Send + Sync {
    /// The universal library's folders.
    fn folders(&self) -> Vec<LibraryFolder>;
    fn save_folders(&self, folders: Vec<LibraryFolder>);
    /// What the user noted on a file; empty when nothing.
    fn data(&self, file: &str) -> FileData;
    fn update_data(&self, file: &str, change: &dyn Fn(FileData) -> FileData) -> FileData;
}

pub type OnError = Arc<dyn Fn(anyhow::Error) + Send + Sync>;

fn log_error() -> OnError {
    Arc::new(|error| tracing::error!("{error:#}"))
}

/// library.json, read once and written whole on each change. A broken file is kept aside, never overwritten.
pub struct FileLibraryStore {
    file: PathBuf,
    on_error: OnError,
    state: Mutex<LibraryJson>,
}

pub fn file_library_store(file: impl Into<PathBuf>) -> FileLibraryStore {
    FileLibraryStore::new(file, log_error())
}

impl FileLibraryStore {
    pub fn new(file: impl Into<PathBuf>, on_error: OnError) -> Self {
        let store = Self {
            file: file.into(),
            on_error,
            state: Mutex::new(LibraryJson::default()),
        };
        *store.state.lock().unwrap() = store.read();
        store
    }

    fn read(&self) -> LibraryJson {
        let empty = LibraryJson {
            version: VERSION,
            ..LibraryJson::default()
        };
        let raw: Value = match std::fs::read_to_string(&self.file) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return empty,
            Err(error) => {
                self.keep_aside(&format!("can't be read ({error})"));
                return empty;
            }
            Ok(text) => match serde_json::from_str(&text) {
                Ok(raw) => raw,
                Err(error) => {
                    self.keep_aside(&format!("can't be read ({error})"));
                    return empty;
                }
            },
        };
        if raw.get("version").and_then(Value::as_u64) != Some(u64::from(VERSION)) {
            self.keep_aside("is from another Linker-Edit version");
            return empty;
        }
        let folders = raw
            .get("folders")
            .and_then(Value::as_array)
            .map(|folders| folders.iter().filter_map(clean_folder).collect())
            .unwrap_or_default();
        let files = raw
            .get("files")
            .and_then(Value::as_object)
            .map(|files| {
                files
                    .iter()
                    .filter_map(|(key, value)| Some((key.clone(), clean_data(value)?)))
                    .collect()
            })
            .unwrap_or_default();
        LibraryJson {
            version: VERSION,
            folders,
            files,
        }
    }

    fn keep_aside(&self, problem: &str) {
        let kept = format!(
            "{}.bad-{}",
            self.file.display(),
            chrono::Utc::now().timestamp_millis()
        );
        match std::fs::rename(&self.file, &kept) {
            Ok(()) => (self.on_error)(anyhow!(
                "{} {problem}. It was kept as {kept}, and the library starts empty.",
                self.file.display()
            )),
            Err(error) => (self.on_error)(error.into()),
        }
    }

    fn save(&self, state: &LibraryJson) {
        let write = || -> anyhow::Result<()> {
            if let Some(dir) = self.file.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let temp = PathBuf::from(format!("{}.tmp", self.file.display()));
            std::fs::write(&temp, format!("{}\n", serde_json::to_string_pretty(state)?))?;
            std::fs::rename(&temp, &self.file)?;
            Ok(())
        };
        if let Err(error) = write() {
            (self.on_error)(error);
        }
    }
}

impl LibraryStore for FileLibraryStore {
    fn folders(&self) -> Vec<LibraryFolder> {
        self.state.lock().unwrap().folders.clone()
    }

    fn save_folders(&self, folders: Vec<LibraryFolder>) {
        let mut state = self.state.lock().unwrap();
        state.folders = folders;
        self.save(&state);
    }

    fn data(&self, file: &str) -> FileData {
        let state = self.state.lock().unwrap();
        state
            .files
            .get(&path_key(file))
            .cloned()
            .unwrap_or_default()
    }

    fn update_data(&self, file: &str, change: &dyn Fn(FileData) -> FileData) -> FileData {
        let key = path_key(file);
        let mut state = self.state.lock().unwrap();
        let next = change(state.files.get(&key).cloned().unwrap_or_default());
        if next.is_empty() {
            state.files.remove(&key);
        } else {
            state.files.insert(key, next.clone());
        }
        self.save(&state);
        next
    }
}

/// A library store in memory, for tests.
#[derive(Default)]
pub struct MemoryLibraryStore {
    folders: Mutex<Vec<LibraryFolder>>,
    files: Mutex<BTreeMap<String, FileData>>,
}

pub fn memory_library_store(folders: Vec<LibraryFolder>) -> MemoryLibraryStore {
    MemoryLibraryStore {
        folders: Mutex::new(folders),
        files: Mutex::default(),
    }
}

impl LibraryStore for MemoryLibraryStore {
    fn folders(&self) -> Vec<LibraryFolder> {
        self.folders.lock().unwrap().clone()
    }

    fn save_folders(&self, folders: Vec<LibraryFolder>) {
        *self.folders.lock().unwrap() = folders;
    }

    fn data(&self, file: &str) -> FileData {
        let files = self.files.lock().unwrap();
        files.get(&path_key(file)).cloned().unwrap_or_default()
    }

    fn update_data(&self, file: &str, change: &dyn Fn(FileData) -> FileData) -> FileData {
        let mut files = self.files.lock().unwrap();
        let key = path_key(file);
        let next = change(files.get(&key).cloned().unwrap_or_default());
        files.insert(key, next.clone());
        next
    }
}

/// A project's library folders, kept in its local.json next to the rest of this machine's link.
pub fn project_folders(local: &LocalLink) -> Vec<LibraryFolder> {
    local
        .library
        .as_ref()
        .and_then(Value::as_array)
        .map(|folders| folders.iter().filter_map(clean_folder).collect())
        .unwrap_or_default()
}

pub fn with_project_folders(local: &LocalLink, folders: &[LibraryFolder]) -> LocalLink {
    let mut next = local.clone();
    next.library = if folders.is_empty() {
        None
    } else {
        Some(serde_json::to_value(folders).unwrap_or_else(|_| Value::Object(Map::new())))
    };
    next
}

/// A new link for a project keeps its library folders, which aren't part of the link.
pub fn keep_library(from: &LocalLink, to: &LocalLink) -> LocalLink {
    with_project_folders(to, &project_folders(from))
}

#[cfg(test)]
mod tests {
    use linker_edit_protocol::ResolveDatabase;
    use serde_json::json;

    use super::*;
    use crate::library::testing::{folder, temp};

    #[test]
    fn keeps_folders_and_whats_noted_on_files_in_library_json_and_puts_a_broken_one_aside() {
        let dir = temp();
        let file = dir.path().join("library.json");
        let store = file_library_store(&file);
        store.save_folders(vec![folder("C:\\Music", "music", "music")]);
        store.update_data("C:\\Music\\Song.wav", &|data| FileData {
            star: true,
            tags: vec!["happy".into()],
            ..data
        });
        let again = file_library_store(&file);
        let resolved = js::resolve("C:\\Music").to_string_lossy().into_owned();
        assert_eq!(again.folders(), vec![folder(&resolved, "music", "music")]);
        let data = again.data("C:\\Music\\Song.wav");
        assert!(data.star);
        assert_eq!(data.tags, ["happy"]);
        // Nothing noted any more: the file's entry goes.
        again.update_data("C:\\Music\\Song.wav", &|data| FileData {
            star: false,
            tags: vec![],
            ..data
        });
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(saved["files"], json!({}));

        std::fs::write(&file, "{ not json").unwrap();
        let errors = Arc::new(Mutex::new(Vec::new()));
        let seen = errors.clone();
        let broken = FileLibraryStore::new(
            &file,
            Arc::new(move |error| seen.lock().unwrap().push(error.to_string())),
        );
        assert_eq!(broken.folders(), vec![]);
        assert_eq!(errors.lock().unwrap().len(), 1);
        assert!(std::fs::read_dir(dir.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("library.json.bad-")
        }));
    }

    #[test]
    fn keeps_a_projects_folders_in_its_local_link_also_when_the_link_changes() {
        let disk = |name: &str| LocalLink {
            database: Some(ResolveDatabase {
                r#type: "Disk".into(),
                name: name.into(),
                host: None,
            }),
            ..LocalLink::default()
        };
        let local = with_project_folders(&disk("Local"), &[folder("D:\\Own", "music", "music")]);
        let ids: Vec<_> = project_folders(&local).into_iter().map(|f| f.id).collect();
        assert_eq!(ids, ["music"]);
        assert_eq!(
            project_folders(&keep_library(&local, &disk("Other"))).len(),
            1
        );
        assert_eq!(with_project_folders(&local, &[]), disk("Local"));
        let nonsense = LocalLink {
            library: Some(json!([{ "id": "x", "kind": "nonsense", "path": "D:\\" }])),
            ..LocalLink::default()
        };
        assert_eq!(project_folders(&nonsense), vec![]);
    }

    #[test]
    fn reads_local_json_with_its_library_and_writes_it_back() {
        let saved = json!({ "database": { "type": "Disk", "name": "Local" }, "library": [{ "id": "a", "kind": "sfx", "label": "Hits", "path": "/sfx", "note": "" }] });
        let local: LocalLink = serde_json::from_value(saved.clone()).unwrap();
        assert_eq!(project_folders(&local).len(), 1);
        assert_eq!(serde_json::to_value(&local).unwrap(), saved);
    }

    #[test]
    fn cleans_up_tags() {
        let tags = clean_tags(&json!(["happy", " Happy ", "intro", "", 3, "two  words"]));
        assert_eq!(tags, ["happy", "intro", "two words"]);
    }
}
