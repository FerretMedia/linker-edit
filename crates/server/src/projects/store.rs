// Projects and their threads on disk: <root>/projects.json lists the project folders, each folder has its
// linker-edit.json (committed, shared with teammates) and .linker-edit/ (this machine's: local.json, threads/).
// The files are the TypeScript server's, byte for byte, so both apps read each other's.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, bail};
use linker_edit_protocol::project_icon::parse_project_icon;
use linker_edit_protocol::{
    ChatItem, ContextUsage, LocalLink, ProjectIcon, Provider, ResolveLink, ThreadSidebarState,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::js;

/// A thread's place in the sidebar: ThreadSidebarState without createdAt, which SavedThread has itself. All ISO
/// 8601; None in files from before them, and for anything that isn't a date.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSidebar {
    #[serde(default, deserialize_with = "date")]
    pub pinned_at: Option<String>,
    #[serde(default, deserialize_with = "date")]
    pub settled_at: Option<String>,
    #[serde(default, deserialize_with = "date")]
    pub unsettled_at: Option<String>,
    #[serde(default, deserialize_with = "date")]
    pub snoozed_until: Option<String>,
    #[serde(default, deserialize_with = "date")]
    pub snoozed_at: Option<String>,
    #[serde(default, deserialize_with = "date")]
    pub last_read_at: Option<String>,
    #[serde(default, deserialize_with = "date")]
    pub last_sent_at: Option<String>,
    #[serde(default, deserialize_with = "date")]
    pub turn_ended_at: Option<String>,
}

/// withSidebarFields: a string that reads as a date, or None.
fn date<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::String(text) if js::is_date(&text) => Some(text),
        _ => None,
    })
}

/// The sidebar fields of a thread that has none yet.
pub const NEW_THREAD_SIDEBAR: ThreadSidebar = ThreadSidebar {
    pinned_at: None,
    settled_at: None,
    unsettled_at: None,
    snoozed_until: None,
    snoozed_at: None,
    last_read_at: None,
    last_sent_at: None,
    turn_ended_at: None,
};

impl ThreadSidebar {
    /// The whole ThreadSidebarState, for a ThreadSummary.
    pub fn with_created_at(&self, created_at: &str) -> ThreadSidebarState {
        ThreadSidebarState {
            created_at: created_at.to_string(),
            pinned_at: self.pinned_at.clone(),
            settled_at: self.settled_at.clone(),
            unsettled_at: self.unsettled_at.clone(),
            snoozed_until: self.snoozed_until.clone(),
            snoozed_at: self.snoozed_at.clone(),
            last_read_at: self.last_read_at.clone(),
            last_sent_at: self.last_sent_at.clone(),
            turn_ended_at: self.turn_ended_at.clone(),
        }
    }
}

/// A thread as kept on disk, so a restart continues it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedThread {
    pub id: String,
    pub title: String,
    pub provider: Provider,
    /// The provider's conversation id; None before the first turn.
    #[serde(default)]
    pub resume_id: Option<String>,
    pub items: Vec<ChatItem>,
    #[serde(default)]
    pub context: Option<ContextUsage>,
    /// ISO 8601.
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub archived_at: Option<String>,
    #[serde(flatten)]
    pub sidebar: ThreadSidebar,
}

/// In linker-edit.json, so a teammate who gets the project sees which workspace or series it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectCollection {
    pub id: String,
    /// As it was when the project was put in it, for computers that don't have it.
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedProject {
    /// This machine's id for the project; it isn't in the project's own files.
    pub id: String,
    pub dir: PathBuf,
    /// None shows the linked Resolve project's name.
    pub name: Option<String>,
    pub link: Option<ResolveLink>,
    pub local: LocalLink,
    /// In linker-edit.json, so a teammate sees the same icon.
    pub icon: Option<ProjectIcon>,
    /// This machine's own sidebar choices, in projects.json.
    pub pinned: bool,
    pub archived: bool,
    /// The workspace or series it is in. None in files from before workspaces.
    pub collection: Option<ProjectCollection>,
    /// Holds its collection's setup threads rather than a Resolve project; see ProjectSummary.setup.
    pub setup: bool,
}

/// What create() makes a project of; the rest starts empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewProject {
    pub name: Option<String>,
    pub link: Option<ResolveLink>,
    pub local: LocalLink,
    pub collection: Option<ProjectCollection>,
    pub setup: bool,
}

/// A project with its threads, as load() and adopt() give them.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedProject {
    pub project: SavedProject,
    pub threads: Vec<SavedThread>,
}

/// Where projects and threads are kept: on disk (FileProjectStore) or, for app tests, in memory
/// (testing::MemoryProjectStore).
pub trait ProjectStore: Send + Sync {
    /// Every project with its threads, in the order they were added.
    fn load(&self) -> Vec<LoadedProject>;
    /// A new folder under the store's root for a new project.
    fn create(&self, project: NewProject) -> SavedProject;
    /// Registers a folder that already has a linker-edit.json, with the threads it has.
    fn adopt(&self, dir: &Path) -> anyhow::Result<LoadedProject>;
    fn save_project(&self, project: &SavedProject);
    /// Only forgets the project; its folder stays.
    fn remove_project(&self, id: &str);
    fn save_thread(&self, project: &SavedProject, thread: &SavedThread);
    fn delete_thread(&self, project: &SavedProject, thread_id: &str);
}

/// The committed file that says which Resolve project a folder belongs to.
pub const PROJECT_FILE: &str = "linker-edit.json";
pub const STATE_DIR: &str = ".linker-edit";

// Bump when a file changes shape; a file with another version is ignored rather than misread.
const VERSION: u32 = 1;

fn is_version(value: &Value) -> bool {
    value.get("version").and_then(Value::as_f64) == Some(f64::from(VERSION))
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// linker-edit.json as it is written.
#[derive(Serialize)]
struct ProjectFile<'a> {
    version: u32,
    name: &'a Option<String>,
    resolve: &'a Option<ResolveLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<&'a ProjectIcon>,
    #[serde(skip_serializing_if = "Option::is_none")]
    collection: Option<&'a ProjectCollection>,
    #[serde(skip_serializing_if = "is_false")]
    setup: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RegistryEntry {
    id: String,
    dir: String,
    /// Missing in files from before the T3 Code sidebar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pinned: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    archived: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Registry {
    version: u32,
    projects: Vec<RegistryEntry>,
}

/// A thread file: .linker-edit/threads/<id>.json.
#[derive(Serialize)]
struct ThreadFile<'a> {
    version: u32,
    thread: &'a SavedThread,
}

/// The one chat Linker-Edit kept before it had threads, in <workspace>/.linker-edit/chat.json.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyChat {
    provider: Provider,
    #[serde(default)]
    resume_id: Option<String>,
    items: Vec<ChatItem>,
    #[serde(default)]
    context: Option<ContextUsage>,
}

pub fn thread_title(items: &[ChatItem]) -> String {
    let first = items.iter().find_map(|item| match item {
        ChatItem::User { text, .. } => Some(text.as_str()),
        _ => None,
    });
    let line = first
        .map(|text| js::trim(js::trim(text).split('\n').next().unwrap_or_default()))
        .unwrap_or_default();
    if line.is_empty() {
        "New thread".into()
    } else if js::length(line) > 80 {
        format!("{}…", js::slice(line, 79))
    } else {
        line.into()
    }
}

// Windows can't name a folder with these, or with a trailing dot or space.
fn folder_name(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c <= '\u{1f}' {
                ' '
            } else {
                c
            }
        })
        .collect();
    let collapsed = replaced
        .split(|c: char| c.is_whitespace() || c == '\u{FEFF}')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let cleaned = collapsed.trim_end_matches(['.', ' ']);
    let cleaned = js::slice(cleaned, 60);
    if cleaned.is_empty() {
        "Project".into()
    } else {
        cleaned.into()
    }
}

/// Reads a JSON file; None when it doesn't exist.
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

/// Written whole and renamed, so a crash mid-write can't leave half a file. As JSON.stringify(value, null, 2).
pub(crate) fn write_json(file: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    write_text(file, &format!("{}\n", js::stringify_pretty(value)))
}

/// Written whole and renamed, so a crash mid-write can't leave half a file.
pub(crate) fn write_text(file: &Path, text: &str) -> anyhow::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut temp = file.as_os_str().to_owned();
    temp.push(".tmp");
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, file)?;
    Ok(())
}

fn uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

type OnError = Arc<dyn Fn(anyhow::Error) + Send + Sync>;

/// Projects registered in <root>/projects.json, each in its own folder. On the first run the old
/// single workspace becomes the first project, in place, and its chat its first thread.
pub struct FileProjectStore {
    root: PathBuf,
    on_error: OnError,
    registry: Mutex<Registry>,
}

/// A FileProjectStore whose problems are logged. legacy_dir is the workspace folder from before projects.
pub fn file_project_store(root: &Path, legacy_dir: &Path) -> anyhow::Result<FileProjectStore> {
    FileProjectStore::new(root, legacy_dir, |error| tracing::error!("{error:#}"))
}

fn state_dir(dir: &Path) -> PathBuf {
    dir.join(STATE_DIR)
}

fn threads_dir(dir: &Path) -> PathBuf {
    state_dir(dir).join("threads")
}

fn thread_file(dir: &Path, id: &str) -> PathBuf {
    threads_dir(dir).join(format!("{id}.json"))
}

fn prepare(dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(state_dir(dir))?;
    std::fs::write(state_dir(dir).join(".gitignore"), "*\n")?;
    Ok(())
}

fn same_dir(a: &Path, b: &Path) -> bool {
    js::resolve(a) == js::resolve(b)
}

impl FileProjectStore {
    pub fn new(
        root: &Path,
        legacy_dir: &Path,
        on_error: impl Fn(anyhow::Error) + Send + Sync + 'static,
    ) -> anyhow::Result<Self> {
        let on_error: OnError = Arc::new(on_error);
        let registry = read_registry(root, legacy_dir, &on_error)?;
        Ok(FileProjectStore {
            root: root.to_path_buf(),
            on_error,
            registry: Mutex::new(registry),
        })
    }

    fn registry_file(&self) -> PathBuf {
        self.root.join("projects.json")
    }

    fn save_registry(&self, registry: &Registry) {
        if let Err(error) = write_json(&self.registry_file(), registry) {
            (self.on_error)(error);
        }
    }

    fn read_project(&self, entry: &RegistryEntry) -> SavedProject {
        let dir = PathBuf::from(&entry.dir);
        let mut file: Option<Value> = None;
        let mut local = LocalLink::default();
        let mut read = || -> anyhow::Result<()> {
            file = read_json(&dir.join(PROJECT_FILE))?.filter(is_version);
            if let Some(saved) = read_json(&state_dir(&dir).join("local.json"))? {
                local = serde_json::from_value(saved).context("local.json isn't a link")?;
            }
            Ok(())
        };
        if let Err(error) = read() {
            (self.on_error)(error);
        }
        let field = |key: &str| file.as_ref().and_then(|file| file.get(key));
        // Files from before workspaces have none.
        let collection = field("collection").and_then(|collection| {
            let id = collection.get("id")?.as_str()?;
            Some(ProjectCollection {
                id: id.to_string(),
                name: collection
                    .get("name")
                    .map(js::to_string)
                    .unwrap_or_default(),
            })
        });
        SavedProject {
            id: entry.id.clone(),
            dir,
            name: field("name").and_then(Value::as_str).map(str::to_string),
            link: field("resolve").and_then(|link| serde_json::from_value(link.clone()).ok()),
            local,
            icon: field("icon").and_then(parse_project_icon),
            pinned: entry.pinned == Some(true),
            archived: entry.archived == Some(true),
            collection,
            setup: field("setup") == Some(&Value::Bool(true)),
        }
    }

    fn migrate_legacy_chat(&self, dir: &Path) -> Option<SavedThread> {
        let file = state_dir(dir).join("chat.json");
        let migrate = || -> anyhow::Result<Option<SavedThread>> {
            let Some(saved) = read_json(&file)? else {
                return Ok(None);
            };
            let chat = saved
                .get("chat")
                .filter(|_| is_version(&saved))
                .and_then(|chat| serde_json::from_value::<LegacyChat>(chat.clone()).ok())
                .filter(|chat| !chat.items.is_empty());
            let Some(chat) = chat else {
                std::fs::remove_file(&file)?;
                return Ok(None);
            };
            let now = js::now_iso();
            let thread = SavedThread {
                id: uuid(),
                title: thread_title(&chat.items),
                provider: chat.provider,
                resume_id: chat.resume_id,
                items: chat.items,
                context: chat.context,
                created_at: now.clone(),
                updated_at: now,
                archived_at: None,
                sidebar: NEW_THREAD_SIDEBAR,
            };
            write_json(
                &thread_file(dir, &thread.id),
                &ThreadFile {
                    version: VERSION,
                    thread: &thread,
                },
            )?;
            std::fs::remove_file(&file)?;
            Ok(Some(thread))
        };
        migrate().unwrap_or_else(|error| {
            (self.on_error)(error);
            None
        })
    }

    fn read_threads(&self, dir: &Path) -> Vec<SavedThread> {
        let mut threads = Vec::new();
        let legacy = self.migrate_legacy_chat(dir);
        let mut names: Vec<String> = Vec::new();
        match std::fs::read_dir(threads_dir(dir)) {
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        Ok(entry) => {
                            let name = entry.file_name().to_string_lossy().into_owned();
                            if name.ends_with(".json") {
                                names.push(name);
                            }
                        }
                        Err(error) => (self.on_error)(error.into()),
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => (self.on_error)(error.into()),
        }
        for name in names {
            let file = threads_dir(dir).join(&name);
            let read = || -> anyhow::Result<Option<SavedThread>> {
                let Some(saved) = read_json(&file)?.filter(is_version) else {
                    return Ok(None);
                };
                match saved.get("thread") {
                    None | Some(Value::Null) => Ok(None),
                    Some(thread) => Ok(Some(
                        serde_json::from_value(thread.clone())
                            .with_context(|| format!("{} isn't a thread", file.display()))?,
                    )),
                }
            };
            match read() {
                Ok(Some(thread)) => threads.push(thread),
                Ok(None) => {}
                Err(error) => (self.on_error)(error),
            }
        }
        if let Some(legacy) = legacy
            && !threads.iter().any(|thread| thread.id == legacy.id)
        {
            threads.push(legacy);
        }
        threads
    }

    fn unique_dir(&self, registry: &Registry, name: &str) -> PathBuf {
        let base = folder_name(name);
        let taken = |dir: &Path| {
            dir.exists()
                || registry
                    .projects
                    .iter()
                    .any(|entry| same_dir(Path::new(&entry.dir), dir))
        };
        let mut dir = self.root.join(&base);
        let mut n = 2;
        while taken(&dir) {
            dir = self.root.join(format!("{base} {n}"));
            n += 1;
        }
        dir
    }

    fn save(&self, registry: &mut Registry, project: &SavedProject) -> anyhow::Result<()> {
        prepare(&project.dir)?;
        let file = ProjectFile {
            version: VERSION,
            name: &project.name,
            resolve: &project.link,
            icon: project.icon.as_ref(),
            collection: project.collection.as_ref(),
            setup: project.setup,
        };
        write_json(&project.dir.join(PROJECT_FILE), &file)?;
        write_json(&state_dir(&project.dir).join("local.json"), &project.local)?;
        if let Some(entry) = registry
            .projects
            .iter_mut()
            .find(|entry| entry.id == project.id)
        {
            if (entry.pinned == Some(true)) != project.pinned {
                entry.pinned = Some(project.pinned);
            }
            if (entry.archived == Some(true)) != project.archived {
                entry.archived = Some(project.archived);
            }
        }
        self.save_registry(registry);
        Ok(())
    }
}

/// The project folders under the root, found by their linker-edit.json.
fn project_folders(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_dir())
                && entry.path().join(PROJECT_FILE).exists()
        })
        .map(|entry| root.join(entry.file_name()))
        .collect()
}

fn read_registry(root: &Path, legacy_dir: &Path, on_error: &OnError) -> anyhow::Result<Registry> {
    let registry_file = root.join("projects.json");
    let problem = match read_json(&registry_file) {
        Ok(None) => None,
        Ok(Some(saved)) => {
            if is_version(&saved) && saved.get("projects").is_some_and(Value::is_array) {
                match serde_json::from_value::<Registry>(saved) {
                    Ok(registry) => return Ok(registry),
                    Err(error) => Some(format!("can't be read ({error})")),
                }
            } else {
                Some("is from another Linker-Edit version".to_string())
            }
        }
        Err(error) => Some(format!("can't be read ({:#})", error.root_cause())),
    };
    if let Some(problem) = &problem {
        // Kept, not overwritten: the list is rebuilt from the project folders, and the old file stays to look at.
        let mut kept = registry_file.as_os_str().to_owned();
        kept.push(format!(".bad-{}", js::now_millis()));
        let kept = PathBuf::from(kept);
        if let Err(error) = std::fs::rename(&registry_file, &kept) {
            on_error(error.into());
        }
        on_error(anyhow::anyhow!(
            "{} {problem}. It was kept as {}, and the project list was rebuilt from the folders in {}.",
            registry_file.display(),
            kept.display(),
            root.display()
        ));
    }
    let mut dirs: Vec<PathBuf> = vec![legacy_dir.to_path_buf()];
    if problem.is_some() {
        dirs.extend(project_folders(root));
    }
    let mut unique: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        if !unique.iter().any(|other| same_dir(other, &dir)) {
            unique.push(dir);
        }
    }
    let rebuilt = Registry {
        version: VERSION,
        projects: unique
            .into_iter()
            .map(|dir| RegistryEntry {
                id: uuid(),
                dir: dir.to_string_lossy().into_owned(),
                pinned: None,
                archived: None,
            })
            .collect(),
    };
    std::fs::create_dir_all(legacy_dir)?;
    write_json(&registry_file, &rebuilt)?;
    Ok(rebuilt)
}

impl ProjectStore for FileProjectStore {
    fn load(&self) -> Vec<LoadedProject> {
        let entries = self.registry.lock().unwrap().projects.clone();
        entries
            .iter()
            .map(|entry| {
                if let Err(error) = prepare(Path::new(&entry.dir)) {
                    (self.on_error)(error);
                }
                LoadedProject {
                    project: self.read_project(entry),
                    threads: self.read_threads(Path::new(&entry.dir)),
                }
            })
            .collect()
    }

    fn create(&self, fields: NewProject) -> SavedProject {
        let mut registry = self.registry.lock().unwrap();
        let name = fields
            .name
            .clone()
            .or_else(|| fields.link.as_ref().map(|link| link.name.clone()))
            .unwrap_or_else(|| "Project".into());
        let project = SavedProject {
            id: uuid(),
            dir: self.unique_dir(&registry, &name),
            name: fields.name,
            link: fields.link,
            local: fields.local,
            icon: None,
            pinned: false,
            archived: false,
            collection: fields.collection,
            setup: fields.setup,
        };
        registry.projects.push(RegistryEntry {
            id: project.id.clone(),
            dir: project.dir.to_string_lossy().into_owned(),
            pinned: None,
            archived: None,
        });
        if let Err(error) = self.save(&mut registry, &project) {
            (self.on_error)(error);
        }
        project
    }

    fn adopt(&self, dir: &Path) -> anyhow::Result<LoadedProject> {
        let resolved = js::resolve(dir);
        if !resolved.join(PROJECT_FILE).exists() {
            bail!(
                "{} has no {PROJECT_FILE}, so it isn't a Linker-Edit project folder.",
                resolved.display()
            );
        }
        let entry = {
            let mut registry = self.registry.lock().unwrap();
            if registry
                .projects
                .iter()
                .any(|entry| js::resolve(Path::new(&entry.dir)) == resolved)
            {
                bail!(
                    "{} is already a project in Linker-Edit.",
                    resolved.display()
                );
            }
            prepare(&resolved)?;
            let entry = RegistryEntry {
                id: uuid(),
                dir: resolved.to_string_lossy().into_owned(),
                pinned: None,
                archived: None,
            };
            registry.projects.push(entry.clone());
            self.save_registry(&registry);
            entry
        };
        Ok(LoadedProject {
            project: self.read_project(&entry),
            threads: self.read_threads(&resolved),
        })
    }

    fn save_project(&self, project: &SavedProject) {
        let mut registry = self.registry.lock().unwrap();
        if let Err(error) = self.save(&mut registry, project) {
            (self.on_error)(error);
        }
    }

    fn remove_project(&self, id: &str) {
        let mut registry = self.registry.lock().unwrap();
        registry.projects.retain(|entry| entry.id != id);
        self.save_registry(&registry);
    }

    fn save_thread(&self, project: &SavedProject, thread: &SavedThread) {
        let save = || -> anyhow::Result<()> {
            prepare(&project.dir)?;
            write_json(
                &thread_file(&project.dir, &thread.id),
                &ThreadFile {
                    version: VERSION,
                    thread,
                },
            )
        };
        if let Err(error) = save() {
            (self.on_error)(error);
        }
    }

    fn delete_thread(&self, project: &SavedProject, thread_id: &str) {
        match std::fs::remove_file(thread_file(&project.dir, thread_id)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                (self.on_error)(error.into())
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    struct Setup {
        _temp: tempfile::TempDir,
        home: PathBuf,
        root: PathBuf,
        legacy_dir: PathBuf,
        errors: Arc<Mutex<Vec<String>>>,
    }

    impl Setup {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let home = temp.path().to_path_buf();
            Setup {
                root: home.join("projects"),
                legacy_dir: home.join("workspace"),
                home,
                _temp: temp,
                errors: Arc::default(),
            }
        }

        fn open(&self) -> FileProjectStore {
            let errors = self.errors.clone();
            FileProjectStore::new(&self.root, &self.legacy_dir, move |error| {
                errors.lock().unwrap().push(format!("{error:#}"))
            })
            .unwrap()
        }

        fn errors(&self) -> Vec<String> {
            self.errors.lock().unwrap().clone()
        }
    }

    fn thread() -> SavedThread {
        SavedThread {
            id: "t1".into(),
            title: "Cut the silences".into(),
            provider: Provider::Codex,
            resume_id: Some("thread-1".into()),
            items: vec![ChatItem::User {
                id: "1".into(),
                text: "Cut the silences".into(),
            }],
            context: None,
            created_at: "2026-11-09T10:00:00.000Z".into(),
            updated_at: "2026-11-09T10:00:00.000Z".into(),
            archived_at: None,
            sidebar: NEW_THREAD_SIDEBAR,
        }
    }

    fn read(file: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap()
    }

    fn new_project(name: Option<&str>, link: Option<ResolveLink>, local: LocalLink) -> NewProject {
        NewProject {
            name: name.map(str::to_string),
            link,
            local,
            ..NewProject::default()
        }
    }

    #[test]
    fn makes_the_old_workspace_the_first_project_with_its_chat_as_the_first_thread() {
        let setup = Setup::new();
        std::fs::create_dir_all(setup.legacy_dir.join(".linker-edit")).unwrap();
        let chat = json!({ "provider": "claude", "resumeId": "abc", "items": [{ "id": "1", "kind": "user", "text": "Rough cut\nof the interview" }], "context": null });
        std::fs::write(
            setup.legacy_dir.join(".linker-edit").join("chat.json"),
            json!({ "version": 1, "chat": chat }).to_string(),
        )
        .unwrap();

        let loaded = setup.open().load();
        assert_eq!(loaded.len(), 1);
        let first = &loaded[0];
        assert_eq!(first.project.dir, setup.legacy_dir);
        assert_eq!((&first.project.name, &first.project.link), (&None, &None));
        assert_eq!(first.project.local, LocalLink::default());
        assert_eq!(first.threads.len(), 1);
        let migrated = &first.threads[0];
        assert_eq!(
            (
                migrated.title.as_str(),
                migrated.provider,
                migrated.resume_id.as_deref(),
                &migrated.archived_at
            ),
            ("Rough cut", Provider::Claude, Some("abc"), &None)
        );
        assert!(
            !setup
                .legacy_dir
                .join(".linker-edit")
                .join("chat.json")
                .exists()
        );
        assert_eq!(
            std::fs::read_to_string(setup.legacy_dir.join(".linker-edit").join(".gitignore"))
                .unwrap(),
            "*\n"
        );

        // The next start reads the registry and the migrated thread, and doesn't register the folder again.
        let again = setup.open().load();
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].project.id, first.project.id);
        assert_eq!(again[0].project.dir, setup.legacy_dir);
        assert_eq!(again[0].threads[0].resume_id.as_deref(), Some("abc"));
        assert!(setup.root.join("projects.json").exists());
    }

    #[test]
    fn creates_a_folder_per_project_keeping_the_link_in_linker_edit_json() {
        let setup = Setup::new();
        let store = setup.open();
        let link = ResolveLink {
            kind: linker_edit_protocol::ResolveLinkKind::Cloud,
            name: "Client: \"X\"".into(),
            project_id: None,
        };
        let a = store.create(new_project(
            None,
            Some(link.clone()),
            LocalLink {
                database: None,
                folder: Some(vec!["Clients".into()]),
                library: None,
            },
        ));
        let b = store.create(new_project(None, Some(link.clone()), LocalLink::default()));
        assert_eq!(
            [&a.dir, &b.dir],
            [&setup.root.join("Client X"), &setup.root.join("Client X 2")]
        );
        assert_eq!(
            read(&a.dir.join("linker-edit.json")),
            json!({ "version": 1, "name": null, "resolve": { "kind": "cloud", "name": "Client: \"X\"", "projectId": null } })
        );

        store.save_thread(&a, &thread());
        store.save_thread(
            &a,
            &SavedThread {
                id: "t2".into(),
                archived_at: Some("2026-11-09T11:00:00.000Z".into()),
                ..thread()
            },
        );
        store.delete_thread(&a, "t2");
        store.remove_project(&b.id);

        let loaded = setup.open().load();
        assert_eq!(loaded.len(), 2);
        assert!(
            loaded[0]
                .project
                .dir
                .to_string_lossy()
                .contains("workspace")
        );
        assert_eq!(
            loaded[1],
            LoadedProject {
                project: a,
                threads: vec![thread()]
            }
        );
        assert!(b.dir.exists());
    }

    #[test]
    fn adopts_a_cloned_project_folder_with_its_link_and_only_once() {
        let setup = Setup::new();
        let store = setup.open();
        let clone = setup.home.join("clone");
        std::fs::create_dir(&clone).unwrap();
        let refused = store.adopt(&clone).unwrap_err().to_string();
        assert!(refused.contains("has no linker-edit.json"), "{refused}");
        let link = json!({ "kind": "cloud", "name": "Client X", "projectId": "cx" });
        std::fs::write(
            clone.join("linker-edit.json"),
            json!({ "version": 1, "name": "Client X edit", "resolve": link }).to_string(),
        )
        .unwrap();

        let adopted = store.adopt(&clone).unwrap();
        assert_eq!(adopted.project.dir, js::resolve(&clone));
        assert_eq!(adopted.project.name.as_deref(), Some("Client X edit"));
        assert_eq!(
            adopted.project.link,
            Some(serde_json::from_value(link).unwrap())
        );
        assert_eq!(adopted.project.local, LocalLink::default());
        assert!(adopted.threads.is_empty());
        let refused = store.adopt(&clone).unwrap_err().to_string();
        assert!(refused.contains("already a project"), "{refused}");
    }

    #[test]
    fn keeps_the_projects_workspace_or_series_and_loads_older_files_without_one() {
        let setup = Setup::new();
        let store = setup.open();
        let old = setup.home.join("old");
        std::fs::create_dir(&old).unwrap();
        std::fs::write(
            old.join("linker-edit.json"),
            json!({ "version": 1, "name": "Old", "resolve": null }).to_string(),
        )
        .unwrap();
        let project = store.adopt(&old).unwrap().project;
        assert_eq!(
            (project.name.as_deref(), &project.link),
            (Some("Old"), &None)
        );
        assert_eq!(project.collection, None);

        let podcast = ProjectCollection {
            id: "c1".into(),
            name: "Podcast".into(),
        };
        store.save_project(&SavedProject {
            collection: Some(podcast.clone()),
            ..project.clone()
        });
        assert_eq!(
            read(&old.join("linker-edit.json")),
            json!({ "version": 1, "name": "Old", "resolve": null, "collection": { "id": "c1", "name": "Podcast" } })
        );
        let reloaded = setup.open().load();
        let found = reloaded
            .iter()
            .find(|entry| entry.project.dir == js::resolve(&old))
            .unwrap();
        assert_eq!(found.project.collection, Some(podcast));

        // Out of it again, the file is as it was.
        store.save_project(&SavedProject {
            collection: None,
            ..project
        });
        assert_eq!(
            read(&old.join("linker-edit.json")),
            json!({ "version": 1, "name": "Old", "resolve": null })
        );
    }

    #[test]
    fn rebuilds_a_damaged_project_list_from_the_folders_keeping_the_damaged_file() {
        let setup = Setup::new();
        let store = setup.open();
        let a = store.create(new_project(Some("Doc"), None, LocalLink::default()));
        store.save_thread(&a, &thread());
        std::fs::write(setup.root.join("projects.json"), r#"{"version": 1, "proj"#).unwrap();

        let loaded = setup.open().load();
        let dirs: Vec<&PathBuf> = loaded.iter().map(|entry| &entry.project.dir).collect();
        assert_eq!(dirs, [&setup.home.join("workspace"), &a.dir]);
        assert_eq!(loaded[1].threads, [thread()]);
        let kept = std::fs::read_dir(&setup.root)
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("projects.json.bad-")
            })
            .count();
        assert_eq!(kept, 1);
        assert!(
            setup.errors()[0].contains("rebuilt from the folders"),
            "{:?}",
            setup.errors()
        );
        // The rebuilt list is a good one: the next start reads it as it is.
        let ids = |loaded: &[LoadedProject]| {
            loaded
                .iter()
                .map(|entry| entry.project.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&setup.open().load()), ids(&loaded));
    }

    #[test]
    fn skips_a_damaged_thread_file_and_says_so() {
        let setup = Setup::new();
        let store = setup.open();
        let project = store.create(new_project(Some("Doc"), None, LocalLink::default()));
        store.save_thread(&project, &thread());
        std::fs::write(
            project
                .dir
                .join(".linker-edit")
                .join("threads")
                .join("t2.json"),
            r#"{"version": 1, "thread": {"#,
        )
        .unwrap();
        assert_eq!(setup.open().load()[1].threads, [thread()]);
        assert_eq!(setup.errors().len(), 1);
    }

    #[test]
    fn keeps_the_sidebar_fields() {
        let setup = Setup::new();
        let store = setup.open();
        let project = store.create(new_project(Some("Promo"), None, LocalLink::default()));
        let sidebar = ThreadSidebar {
            pinned_at: Some("2026-11-10T09:00:00.000Z".into()),
            settled_at: Some("2026-11-11T09:00:00.000Z".into()),
            unsettled_at: None,
            snoozed_until: Some("2026-11-12T09:00:00.000Z".into()),
            snoozed_at: Some("2026-11-11T08:00:00.000Z".into()),
            last_read_at: Some("2026-11-11T07:00:00.000Z".into()),
            last_sent_at: Some("2026-11-11T06:00:00.000Z".into()),
            turn_ended_at: Some("2026-11-11T06:30:00.000Z".into()),
        };
        let with_sidebar = SavedThread {
            sidebar: sidebar.clone(),
            ..thread()
        };
        store.save_thread(&project, &with_sidebar);
        let icon = ProjectIcon::Emoji {
            emoji: "🎬".into()
        };
        store.save_project(&SavedProject {
            icon: Some(icon.clone()),
            pinned: true,
            archived: true,
            ..project.clone()
        });

        let loaded = setup.open().load().remove(1);
        assert_eq!(loaded.threads, [with_sidebar]);
        assert_eq!(
            (
                &loaded.project.icon,
                loaded.project.pinned,
                loaded.project.archived
            ),
            (&Some(icon), true, true)
        );
        // The icon goes with the project; the pin and archive are this machine's.
        assert_eq!(
            read(&project.dir.join("linker-edit.json"))["icon"],
            json!({ "kind": "emoji", "emoji": "🎬" })
        );
        let registry = read(&setup.root.join("projects.json"));
        assert_eq!(registry["projects"][1]["pinned"], json!(true));
        assert_eq!(registry["projects"][1]["archived"], json!(true));
    }

    #[test]
    fn loads_files_from_before_the_sidebar_fields_and_ignores_a_malformed_icon() {
        let setup = Setup::new();
        let store = setup.open();
        let project = store.create(new_project(Some("Old"), None, LocalLink::default()));
        let old = json!({ "id": "t1", "title": "Old thread", "provider": "claude", "resumeId": null, "items": [], "context": null, "createdAt": "2026-11-01T10:00:00.000Z", "updatedAt": "2026-11-01T10:00:00.000Z", "archivedAt": null });
        let threads = project.dir.join(".linker-edit").join("threads");
        std::fs::create_dir_all(&threads).unwrap();
        let mut with_bad_pin = old.clone();
        with_bad_pin["pinnedAt"] = json!("not a date");
        std::fs::write(
            threads.join("t1.json"),
            json!({ "version": 1, "thread": with_bad_pin }).to_string(),
        )
        .unwrap();
        std::fs::write(
            project.dir.join("linker-edit.json"),
            json!({ "version": 1, "name": "Old", "resolve": null, "icon": { "kind": "lucide", "name": "rocket", "color": "plaid" } }).to_string(),
        )
        .unwrap();

        let loaded = setup.open().load().remove(1);
        let expected: SavedThread = serde_json::from_value(old).unwrap();
        assert_eq!(expected.sidebar, NEW_THREAD_SIDEBAR);
        assert_eq!(loaded.threads, [expected]);
        assert_eq!(loaded.project.name.as_deref(), Some("Old"));
        assert_eq!(
            (
                &loaded.project.icon,
                loaded.project.pinned,
                loaded.project.archived
            ),
            (&None, false, false)
        );
    }

    #[test]
    fn titles_a_thread_by_its_first_message() {
        let user = |text: &str| ChatItem::User {
            id: "1".into(),
            text: text.into(),
        };
        assert_eq!(thread_title(&[]), "New thread");
        assert_eq!(thread_title(&[user("  Rough cut\nof it")]), "Rough cut");
        assert_eq!(
            thread_title(&[user(&"x".repeat(100))]),
            format!("{}…", "x".repeat(79))
        );
        assert_eq!(folder_name("Client: \"X\"."), "Client X");
        assert_eq!(folder_name(" ?? "), "Project");
    }
}
