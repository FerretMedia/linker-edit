// Workspaces and series on disk: <root>/collections.json lists them, each in <root>/collections/<slug>/ with its
// collection.json, RULES.md and examples/. The files are the TypeScript server's, so both apps read each other's.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use anyhow::{Context, anyhow};
use linker_edit_protocol::{
    ApprovedEdit, CollectionExample, CollectionIcon, CollectionKind, Lesson,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

use crate::js;
use crate::projects::store::{write_json, write_text};

/// A workspace or series as kept on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedCollection {
    pub id: String,
    pub kind: CollectionKind,
    pub name: String,
    /// The workspace a series is in; None for a workspace.
    pub parent_id: Option<String>,
    pub icon: CollectionIcon,
    /// RULES.md: the style guide for the agent.
    pub rules: String,
    /// Library folders of a workspace or series. Empty until the library comes to collections; kept as they are.
    pub library: Vec<Value>,
    /// Licensing: its projects only use its own music. Not used yet.
    pub music_only_from_here: bool,
    /// Its projects' agent makes the first cut without asking.
    pub autonomous: bool,
    /// What feedback taught the agent, oldest first.
    pub lessons: Vec<Lesson>,
    /// Approved edits of its projects, newest first.
    pub edits: Vec<ApprovedEdit>,
}

/// What create() makes a collection of; the rest starts empty (NEW_COLLECTION in the TypeScript).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCollection {
    pub kind: CollectionKind,
    pub name: String,
    pub parent_id: Option<String>,
    pub icon: CollectionIcon,
    pub rules: String,
}

impl NewCollection {
    pub fn with_id(self, id: String) -> SavedCollection {
        SavedCollection {
            id,
            kind: self.kind,
            name: self.name,
            parent_id: self.parent_id,
            icon: self.icon,
            rules: self.rules,
            library: Vec::new(),
            music_only_from_here: false,
            autonomous: false,
            lessons: Vec::new(),
            edits: Vec::new(),
        }
    }
}

/// Where workspaces and series are kept: on disk (FileCollectionStore) or, for app tests, in memory
/// (testing::MemoryCollectionStore).
pub trait CollectionStore: Send + Sync {
    /// Every collection that could be read, in the order they were made.
    fn load(&self) -> Vec<SavedCollection>;
    fn create(&self, fields: NewCollection) -> anyhow::Result<SavedCollection>;
    fn save(&self, collection: &SavedCollection);
    /// Takes it out of the list and deletes its folder, rules included.
    fn remove(&self, id: &str);
    /// Where its RULES.md is; None when it isn't in the list.
    fn rules_file(&self, id: &str) -> Option<String>;
    /// The agent's notes on example videos, one Markdown file each in its examples/ folder.
    fn examples(&self, id: &str) -> Vec<CollectionExample>;
    /// Writes or replaces the notes on one example; where they are, or None when the collection is gone.
    fn save_example(&self, id: &str, name: &str, text: &str) -> anyhow::Result<Option<String>>;
    fn remove_example(&self, id: &str, name: &str);
}

pub const COLLECTION_FILE: &str = "collection.json";
pub const RULES_FILE: &str = "RULES.md";
pub const EXAMPLES_DIR: &str = "examples";

static VIDEO_EXTENSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\.[a-z0-9]{2,5}$").unwrap());

/// "Reel 12 - launch.mp4" → "reel-12-launch": an example's notes are named after its video.
pub fn example_name(name: &str) -> String {
    collection_slug(&VIDEO_EXTENSION.replace(name, ""))
}

// Bump when a file changes shape; a file with another version is left alone rather than misread.
const VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RegistryEntry {
    id: String,
    /// The name under collections/.
    folder: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Registry {
    version: u32,
    /// In the order they were made.
    collections: Vec<RegistryEntry>,
}

/// collection.json: everything but the rules, which have their own file for editing by hand.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CollectionFile<'a> {
    version: u32,
    id: &'a str,
    kind: CollectionKind,
    name: &'a str,
    parent_id: &'a Option<String>,
    icon: &'a CollectionIcon,
    library: &'a [Value],
    music_only_from_here: bool,
    autonomous: bool,
    lessons: &'a [Lesson],
    edits: &'a [ApprovedEdit],
}

/// collection.json as read; autonomous, lessons and edits are missing in files from before the agent learned brands.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedFile {
    kind: CollectionKind,
    name: String,
    #[serde(default)]
    parent_id: Option<String>,
    icon: CollectionIcon,
    #[serde(default)]
    library: Value,
    #[serde(default)]
    music_only_from_here: Value,
    #[serde(default)]
    autonomous: Value,
    #[serde(default)]
    lessons: Value,
    #[serde(default)]
    edits: Value,
}

/// An array field as it is, or empty when it's anything else.
fn list<T: serde::de::DeserializeOwned>(value: Value) -> Vec<T> {
    if value.is_array() {
        serde_json::from_value(value).unwrap_or_default()
    } else {
        Vec::new()
    }
}

/// "Ferret Media" → "ferret-media", safe as a folder name everywhere.
pub fn collection_slug(name: &str) -> String {
    let decomposed: String = name
        .nfkd()
        .filter(|c| !('\u{300}'..='\u{36f}').contains(c))
        .collect::<String>()
        .to_lowercase();
    let mut slug = String::new();
    let mut dash = false;
    for c in decomposed.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if dash && !slug.is_empty() {
                slug.push('-');
            }
            dash = false;
            slug.push(c);
        } else {
            dash = true;
        }
    }
    // Leading dashes never get in; a trailing one is only pushed before a letter. Now the length, then its dashes.
    let mut slug: String = slug.chars().take(50).collect();
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "collection".into()
    } else {
        slug
    }
}

fn read_json(file: &Path) -> anyhow::Result<Option<Value>> {
    match std::fs::read_to_string(file) {
        Ok(text) => {
            Ok(Some(serde_json::from_str(&text).with_context(|| {
                format!("Couldn't read {}", file.display())
            })?))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            Err(anyhow::Error::new(error).context(format!("Couldn't read {}", file.display())))
        }
    }
}

fn is_version(value: &Value) -> bool {
    value.get("version").and_then(Value::as_f64) == Some(f64::from(VERSION))
}

type OnError = Arc<dyn Fn(anyhow::Error) + Send + Sync>;

struct State {
    registry: Registry,
    /// Collections whose collection.json couldn't be read: left as they are, never written over.
    unreadable: HashSet<String>,
}

/// Workspaces and series, listed in <root>/collections.json, each in <root>/collections/<slug>/ with its
/// collection.json and RULES.md. A file that can't be read is kept as it is, never overwritten.
pub struct FileCollectionStore {
    root: PathBuf,
    on_error: OnError,
    state: Mutex<State>,
}

/// A FileCollectionStore whose problems are logged.
pub fn file_collection_store(root: &Path) -> FileCollectionStore {
    FileCollectionStore::new(root, |error| tracing::error!("{error:#}"))
}

impl FileCollectionStore {
    pub fn new(root: &Path, on_error: impl Fn(anyhow::Error) + Send + Sync + 'static) -> Self {
        let store = FileCollectionStore {
            root: root.to_path_buf(),
            on_error: Arc::new(on_error),
            state: Mutex::new(State {
                registry: Registry {
                    version: VERSION,
                    collections: Vec::new(),
                },
                unreadable: HashSet::new(),
            }),
        };
        let registry = store.read_registry();
        store.state.lock().unwrap().registry = registry;
        store
    }

    fn registry_file(&self) -> PathBuf {
        self.root.join("collections.json")
    }

    fn folders_dir(&self) -> PathBuf {
        self.root.join("collections")
    }

    fn dir_of(&self, folder: &str) -> PathBuf {
        self.folders_dir().join(folder)
    }

    fn read_registry(&self) -> Registry {
        let registry_file = self.registry_file();
        let problem = match read_json(&registry_file) {
            Ok(None) => {
                return Registry {
                    version: VERSION,
                    collections: Vec::new(),
                };
            }
            Ok(Some(saved)) => {
                if is_version(&saved) && saved.get("collections").is_some_and(Value::is_array) {
                    match serde_json::from_value::<Registry>(saved) {
                        Ok(registry) => return registry,
                        Err(error) => format!("can't be read ({error})"),
                    }
                } else {
                    "is from another Linker-Edit version".to_string()
                }
            }
            Err(error) => format!("can't be read ({:#})", error.root_cause()),
        };
        // Kept, not overwritten: the list is rebuilt from the folders, and the old file stays to look at.
        let mut kept = registry_file.as_os_str().to_owned();
        kept.push(format!(".bad-{}", js::now_millis()));
        let kept = PathBuf::from(kept);
        if let Err(error) = std::fs::rename(&registry_file, &kept) {
            (self.on_error)(error.into());
        }
        (self.on_error)(anyhow!(
            "{} {problem}. It was kept as {}, and the list was rebuilt from the folders in {}.",
            registry_file.display(),
            kept.display(),
            self.folders_dir().display()
        ));
        let mut rebuilt = Registry {
            version: VERSION,
            collections: Vec::new(),
        };
        for folder in self.collection_folders() {
            match read_json(&self.dir_of(&folder).join(COLLECTION_FILE)) {
                Ok(Some(file)) if is_version(&file) => {
                    if let Some(id) = file.get("id").and_then(Value::as_str) {
                        rebuilt.collections.push(RegistryEntry {
                            id: id.to_string(),
                            folder,
                        });
                    }
                }
                Ok(_) => {}
                Err(error) => (self.on_error)(error),
            }
        }
        self.save_registry(&rebuilt);
        rebuilt
    }

    fn collection_folders(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.folders_dir()) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter(|entry| {
                entry.file_type().is_ok_and(|kind| kind.is_dir())
                    && entry.path().join(COLLECTION_FILE).exists()
            })
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect()
    }

    fn save_registry(&self, registry: &Registry) {
        if let Err(error) = write_json(&self.registry_file(), registry) {
            (self.on_error)(error);
        }
    }

    fn read(&self, id: &str, folder: &str) -> anyhow::Result<SavedCollection> {
        let file = self.dir_of(folder).join(COLLECTION_FILE);
        let Some(saved) = read_json(&file)? else {
            return Err(anyhow!(
                "{} is missing, so this workspace or series was left out.",
                file.display()
            ));
        };
        if !is_version(&saved) || saved.get("id").and_then(Value::as_str) != Some(id) {
            return Err(anyhow!(
                "{} is from another Linker-Edit version or another list, so it was left out and left as it is.",
                file.display()
            ));
        }
        let saved: SavedFile = serde_json::from_value(saved)
            .with_context(|| format!("{} isn't a workspace or series", file.display()))?;
        let rules = match std::fs::read_to_string(self.dir_of(folder).join(RULES_FILE)) {
            Ok(rules) => rules,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error.into()),
        };
        Ok(SavedCollection {
            id: id.to_string(),
            kind: saved.kind,
            name: saved.name,
            parent_id: saved.parent_id,
            icon: saved.icon,
            rules,
            library: list(saved.library),
            music_only_from_here: saved.music_only_from_here == Value::Bool(true),
            autonomous: saved.autonomous == Value::Bool(true),
            lessons: list(saved.lessons),
            edits: list(saved.edits),
        })
    }

    fn write(&self, folder: &str, collection: &SavedCollection) -> anyhow::Result<()> {
        let file = CollectionFile {
            version: VERSION,
            id: &collection.id,
            kind: collection.kind,
            name: &collection.name,
            parent_id: &collection.parent_id,
            icon: &collection.icon,
            library: &collection.library,
            music_only_from_here: collection.music_only_from_here,
            autonomous: collection.autonomous,
            lessons: &collection.lessons,
            edits: &collection.edits,
        };
        write_json(&self.dir_of(folder).join(COLLECTION_FILE), &file)?;
        write_text(&self.dir_of(folder).join(RULES_FILE), &collection.rules)
    }

    fn unique_folder(&self, registry: &Registry, name: &str) -> String {
        let base = collection_slug(name);
        let taken = |folder: &str| {
            self.dir_of(folder).exists()
                || registry
                    .collections
                    .iter()
                    .any(|entry| entry.folder.to_lowercase() == folder.to_lowercase())
        };
        let mut folder = base.clone();
        let mut n = 2;
        while taken(&folder) {
            folder = format!("{base}-{n}");
            n += 1;
        }
        folder
    }

    fn folder_of(&self, id: &str) -> Option<String> {
        self.state
            .lock()
            .unwrap()
            .registry
            .collections
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.folder.clone())
    }
}

impl CollectionStore for FileCollectionStore {
    fn load(&self) -> Vec<SavedCollection> {
        let entries = self.state.lock().unwrap().registry.collections.clone();
        entries
            .iter()
            .filter_map(|entry| match self.read(&entry.id, &entry.folder) {
                Ok(collection) => Some(collection),
                Err(error) => {
                    self.state
                        .lock()
                        .unwrap()
                        .unreadable
                        .insert(entry.id.clone());
                    (self.on_error)(error);
                    None
                }
            })
            .collect()
    }

    fn create(&self, fields: NewCollection) -> anyhow::Result<SavedCollection> {
        let mut state = self.state.lock().unwrap();
        let collection = fields.clone().with_id(uuid::Uuid::new_v4().to_string());
        let folder = self.unique_folder(&state.registry, &fields.name);
        self.write(&folder, &collection)?;
        state.registry.collections.push(RegistryEntry {
            id: collection.id.clone(),
            folder,
        });
        self.save_registry(&state.registry);
        Ok(collection)
    }

    fn save(&self, collection: &SavedCollection) {
        let Some(folder) = self.folder_of(&collection.id) else {
            return;
        };
        if self
            .state
            .lock()
            .unwrap()
            .unreadable
            .contains(&collection.id)
        {
            return;
        }
        if let Err(error) = self.write(&folder, collection) {
            (self.on_error)(error);
        }
    }

    fn remove(&self, id: &str) {
        let folder = {
            let mut state = self.state.lock().unwrap();
            let Some(folder) = state
                .registry
                .collections
                .iter()
                .find(|entry| entry.id == id)
                .map(|entry| entry.folder.clone())
            else {
                return;
            };
            state.registry.collections.retain(|entry| entry.id != id);
            self.save_registry(&state.registry);
            folder
        };
        match std::fs::remove_dir_all(self.dir_of(&folder)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                (self.on_error)(error.into())
            }
            _ => {}
        }
    }

    fn rules_file(&self, id: &str) -> Option<String> {
        let folder = self.folder_of(id)?;
        Some(
            self.dir_of(&folder)
                .join(RULES_FILE)
                .to_string_lossy()
                .into_owned(),
        )
    }

    fn examples(&self, id: &str) -> Vec<CollectionExample> {
        let Some(folder) = self.folder_of(id) else {
            return Vec::new();
        };
        let dir = self.dir_of(&folder).join(EXAMPLES_DIR);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".md"))
            .collect();
        names.sort();
        names
            .into_iter()
            .map(|name| CollectionExample {
                file: dir.join(&name).to_string_lossy().into_owned(),
                name: name[..name.len() - 3].to_string(),
            })
            .collect()
    }

    fn save_example(&self, id: &str, name: &str, text: &str) -> anyhow::Result<Option<String>> {
        let Some(folder) = self.folder_of(id) else {
            return Ok(None);
        };
        let file = self
            .dir_of(&folder)
            .join(EXAMPLES_DIR)
            .join(format!("{}.md", example_name(name)));
        write_text(&file, text)?;
        Ok(Some(file.to_string_lossy().into_owned()))
    }

    fn remove_example(&self, id: &str, name: &str) {
        let Some(folder) = self.folder_of(id) else {
            return;
        };
        let file = self
            .dir_of(&folder)
            .join(EXAMPLES_DIR)
            .join(format!("{}.md", example_name(name)));
        match std::fs::remove_file(file) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                (self.on_error)(error.into())
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use linker_edit_protocol::IconColor;
    use serde_json::json;

    use super::*;

    struct Setup {
        _temp: tempfile::TempDir,
        root: PathBuf,
        errors: Arc<Mutex<Vec<String>>>,
    }

    impl Setup {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            Setup {
                root: temp.path().to_path_buf(),
                _temp: temp,
                errors: Arc::default(),
            }
        }

        fn open(&self) -> FileCollectionStore {
            let errors = self.errors.clone();
            FileCollectionStore::new(&self.root, move |error| {
                errors.lock().unwrap().push(format!("{error:#}"))
            })
        }

        fn errors(&self) -> Vec<String> {
            self.errors.lock().unwrap().clone()
        }
    }

    fn monogram() -> CollectionIcon {
        CollectionIcon::Monogram {
            text: String::new(),
            color: IconColor::Blue,
        }
    }

    fn new(
        kind: CollectionKind,
        name: &str,
        parent_id: Option<&str>,
        icon: CollectionIcon,
        rules: &str,
    ) -> NewCollection {
        NewCollection {
            kind,
            name: name.into(),
            parent_id: parent_id.map(str::to_string),
            icon,
            rules: rules.into(),
        }
    }

    fn read(file: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap()
    }

    fn names_in(dir: &Path, prefix: &str) -> usize {
        std::fs::read_dir(dir)
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(prefix)
            })
            .count()
    }

    #[test]
    fn keeps_the_list_each_collections_file_and_its_rules() {
        let setup = Setup::new();
        let store = setup.open();
        let workspace = store
            .create(new(
                CollectionKind::Workspace,
                "Ferret Media",
                None,
                monogram(),
                "",
            ))
            .unwrap();
        let series = store
            .create(new(
                CollectionKind::Series,
                "Podcast",
                Some(&workspace.id),
                CollectionIcon::Lucide {
                    name: "mic".into(),
                    color: IconColor::Violet,
                },
                "Lower thirds in Inter Bold.\n",
            ))
            .unwrap();

        assert_eq!(
            read(&setup.root.join("collections.json")),
            json!({
                "version": 1,
                "collections": [
                    { "id": workspace.id, "folder": "ferret-media" },
                    { "id": series.id, "folder": "podcast" },
                ],
            })
        );
        let dir = setup.root.join("collections").join("podcast");
        assert_eq!(
            read(&dir.join("collection.json")),
            json!({
                "version": 1,
                "id": series.id,
                "kind": "series",
                "name": "Podcast",
                "parentId": workspace.id,
                "icon": { "kind": "lucide", "name": "mic", "color": "violet" },
                "library": [],
                "musicOnlyFromHere": false,
                "autonomous": false,
                "lessons": [],
                "edits": [],
            })
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("RULES.md")).unwrap(),
            "Lower thirds in Inter Bold.\n"
        );
        assert_eq!(
            store.rules_file(&series.id),
            Some(dir.join("RULES.md").to_string_lossy().into_owned())
        );

        // A file from before lessons and working alone loads with neither.
        let file = dir.join("collection.json");
        let mut older = read(&file);
        for key in ["autonomous", "lessons", "edits"] {
            older.as_object_mut().unwrap().remove(key);
        }
        std::fs::write(&file, older.to_string()).unwrap();
        assert_eq!(setup.open().load()[1], series);

        // RULES.md edited in another editor shows on the next start.
        std::fs::write(dir.join("RULES.md"), "End on the logo.").unwrap();
        assert_eq!(
            setup.open().load(),
            [
                workspace,
                SavedCollection {
                    rules: "End on the logo.".into(),
                    ..series
                }
            ]
        );
    }

    #[test]
    fn gives_each_collection_its_own_folder_and_deletes_it_with_the_collection() {
        let setup = Setup::new();
        let store = setup.open();
        let a = store
            .create(new(
                CollectionKind::Workspace,
                "Café Ünïcode!",
                None,
                monogram(),
                "",
            ))
            .unwrap();
        let b = store
            .create(new(
                CollectionKind::Workspace,
                "cafe unicode",
                None,
                monogram(),
                "x",
            ))
            .unwrap();
        let mut folders: Vec<String> = std::fs::read_dir(setup.root.join("collections"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        folders.sort();
        assert_eq!(folders, ["cafe-unicode", "cafe-unicode-2"]);

        let lesson = Lesson {
            id: "l1".into(),
            rule: "Intro at most 3 seconds.".into(),
            feedback: "too slow at the start".into(),
            added_at: "2026-11-17T10:00:00.000Z".into(),
            project: Some("Ep 12".into()),
        };
        store.save(&SavedCollection {
            name: "Renamed".into(),
            rules: "New rules".into(),
            autonomous: true,
            lessons: vec![lesson.clone()],
            ..a.clone()
        });
        let loaded = setup.open().load().remove(0);
        assert_eq!(
            (
                loaded.name.as_str(),
                loaded.rules.as_str(),
                loaded.autonomous,
                &loaded.lessons
            ),
            ("Renamed", "New rules", true, &vec![lesson])
        );

        // Notes on example videos are files next to the rules, named after the video.
        let file = store
            .save_example(&a.id, "Reel 12 - Launch!.mp4", "# Reel 12\n")
            .unwrap();
        let expected = setup
            .root
            .join("collections")
            .join("cafe-unicode")
            .join("examples")
            .join("reel-12-launch.md")
            .to_string_lossy()
            .into_owned();
        assert_eq!(file.as_deref(), Some(expected.as_str()));
        assert_eq!(
            setup.open().examples(&a.id),
            [CollectionExample {
                name: "reel-12-launch".into(),
                file: expected
            }]
        );
        store.remove_example(&a.id, "reel-12-launch");
        assert_eq!(store.examples(&a.id), []);
        assert_eq!(store.save_example("gone", "x", "y").unwrap(), None);

        store.remove(&b.id);
        assert!(
            !setup
                .root
                .join("collections")
                .join("cafe-unicode-2")
                .exists()
        );
        let ids: Vec<String> = setup
            .open()
            .load()
            .into_iter()
            .map(|collection| collection.id)
            .collect();
        assert_eq!(ids, [a.id]);
    }

    #[test]
    fn rebuilds_a_damaged_list_from_the_folders_keeping_the_damaged_file() {
        let setup = Setup::new();
        let store = setup.open();
        let a = store
            .create(new(
                CollectionKind::Workspace,
                "Ferret Media",
                None,
                monogram(),
                "Be brief.",
            ))
            .unwrap();
        std::fs::write(
            setup.root.join("collections.json"),
            r#"{"version": 1, "coll"#,
        )
        .unwrap();

        assert_eq!(setup.open().load(), std::slice::from_ref(&a));
        assert_eq!(names_in(&setup.root, "collections.json.bad-"), 1);
        assert!(
            setup.errors()[0].contains("rebuilt from the folders"),
            "{:?}",
            setup.errors()
        );
        assert_eq!(setup.open().load(), [a]);
    }

    #[test]
    fn leaves_a_damaged_collection_json_out_and_never_writes_over_it() {
        let setup = Setup::new();
        let store = setup.open();
        let a = store
            .create(new(CollectionKind::Workspace, "Good", None, monogram(), ""))
            .unwrap();
        let b = store
            .create(new(
                CollectionKind::Workspace,
                "Broken",
                None,
                monogram(),
                "",
            ))
            .unwrap();
        let broken = setup
            .root
            .join("collections")
            .join("broken")
            .join("collection.json");
        std::fs::write(&broken, r#"{"version": 1, "id""#).unwrap();

        let reopened = setup.open();
        assert_eq!(reopened.load(), [a]);
        assert_eq!(setup.errors().len(), 1);
        // Saving or making another one with the same name doesn't touch it.
        reopened.save(&SavedCollection {
            name: "Broken again".into(),
            ..b
        });
        let c = reopened
            .create(new(
                CollectionKind::Workspace,
                "Broken",
                None,
                monogram(),
                "",
            ))
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&broken).unwrap(),
            r#"{"version": 1, "id""#
        );
        assert_eq!(
            reopened.rules_file(&c.id),
            Some(
                setup
                    .root
                    .join("collections")
                    .join("broken-2")
                    .join("RULES.md")
                    .to_string_lossy()
                    .into_owned()
            )
        );
    }

    #[test]
    fn makes_a_folder_name_of_any_name() {
        assert_eq!(
            collection_slug("  Ferret / Media: 2026  "),
            "ferret-media-2026"
        );
        assert_eq!(collection_slug("🎙"), "collection");
        assert_eq!(collection_slug(&"a".repeat(80)).len(), 50);
        assert_eq!(collection_slug("ab-cd"), "ab-cd");
        assert_eq!(example_name("Reel 12 - Launch!.MP4"), "reel-12-launch");
    }
}
