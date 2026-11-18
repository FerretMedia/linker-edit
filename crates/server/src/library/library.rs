// The library as a whole: its folders' scans, what the user noted on each file, and finding files across a chain
// of libraries. Every lookup takes the chain (universal first, the project last), so more libraries in between
// (workspaces, series) only add links to it.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use linker_edit_protocol::{
    LibraryFile, LibraryFolder, LibraryFolderStatus, LibraryQuery, LibraryScope, LibraryScopeKind,
    LibraryUse, ScanProgress,
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::folders::{FileData, LibraryStore, OnError, path_key};
use super::scan::{
    FolderCache, ProbeFile, ScanOptions, ScannedFile, in_folder, read_cache, remove_cache,
    scan_folder,
};
use super::search::search_library;
use super::serve::is_inside;
use crate::js::{self, Serial};

/// One library in a chain: which one, its name, and its folders.
#[derive(Debug, Clone, PartialEq)]
pub struct LibraryLink {
    pub scope: LibraryScope,
    pub label: String,
    pub folders: Vec<LibraryFolder>,
}

pub fn scope_kind(scope: &LibraryScope) -> LibraryScopeKind {
    match scope {
        LibraryScope::Universal => LibraryScopeKind::Universal,
        LibraryScope::Project { .. } => LibraryScopeKind::Project,
    }
}

pub struct LibraryOptions {
    pub store: Arc<dyn LibraryStore>,
    pub cache_dir: PathBuf,
    pub probe: Arc<dyn ProbeFile>,
    /// Logs with tracing when left out.
    pub on_error: Option<OnError>,
}

/// What the user changes on a file from the library page; None leaves it as it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileChange {
    pub tags: Option<Vec<String>>,
    pub star: Option<bool>,
    pub note: Option<String>,
}

// Progress is sent this often at most while a scan runs.
const PROGRESS: Duration = Duration::from_millis(400);

type Listener = Arc<dyn Fn() + Send + Sync>;

struct Scanning {
    done: u32,
    total: u32,
    cancel: CancellationToken,
}

struct Queued {
    path: String,
    cancelled: AtomicBool,
    done: watch::Receiver<bool>,
}

#[derive(Default)]
struct State {
    caches: HashMap<String, Option<Arc<FolderCache>>>,
    scanning: HashMap<String, Scanning>,
    problems: HashMap<String, String>,
    queued: HashMap<String, Arc<Queued>>,
    progress_timer: bool,
    progress_generation: u64,
}

struct Inner {
    store: Arc<dyn LibraryStore>,
    cache_dir: PathBuf,
    probe: Arc<dyn ProbeFile>,
    on_error: OnError,
    state: Mutex<State>,
    listeners: Mutex<Vec<Listener>>,
    // One folder is scanned at a time, so a library of big folders doesn't run ffmpeg dozens of times at once.
    queue: Serial,
}

/// The library service the app holds: cheap to clone, all clones the same library.
#[derive(Clone)]
pub struct Library {
    inner: Arc<Inner>,
}

impl Inner {
    fn notify(&self) {
        let listeners = self.listeners.lock().unwrap().clone();
        for listener in listeners {
            listener();
        }
    }

    fn changed(&self) {
        {
            let mut state = self.state.lock().unwrap();
            state.progress_timer = false;
            state.progress_generation += 1;
        }
        self.notify();
    }

    fn progressed(self: &Arc<Self>) {
        let generation = {
            let mut state = self.state.lock().unwrap();
            if state.progress_timer {
                return;
            }
            state.progress_timer = true;
            state.progress_generation
        };
        let inner = Arc::downgrade(self);
        tokio::spawn(async move {
            tokio::time::sleep(PROGRESS).await;
            let Some(inner) = Weak::upgrade(&inner) else {
                return;
            };
            {
                let mut state = inner.state.lock().unwrap();
                if !state.progress_timer || state.progress_generation != generation {
                    return;
                }
                state.progress_timer = false;
            }
            inner.notify();
        });
    }

    fn cache_of(&self, state: &mut State, folder: &LibraryFolder) -> Option<Arc<FolderCache>> {
        let cache = state
            .caches
            .entry(folder.id.clone())
            .or_insert_with(|| read_cache(&self.cache_dir, folder).map(Arc::new))
            .clone();
        cache.filter(|cache| cache.path == folder.path)
    }

    fn file_of(
        link: &LibraryLink,
        folder: &LibraryFolder,
        scanned: &ScannedFile,
        data: FileData,
    ) -> LibraryFile {
        LibraryFile {
            folder_id: folder.id.clone(),
            scope: scope_kind(&link.scope),
            kind: folder.kind,
            rel: scanned.rel.clone(),
            name: scanned
                .rel
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_string(),
            path: in_folder(folder, &scanned.rel)
                .to_string_lossy()
                .into_owned(),
            size: scanned.size,
            modified: js::iso_from_ms(scanned.mtime_ms as i64),
            duration: scanned.duration,
            channels: scanned.channels,
            loudness: scanned.loudness,
            tags: data.tags,
            star: data.star,
            note: data.note,
            used_in: data.used_in,
        }
    }

    async fn run_scan(self: Arc<Self>, folder: LibraryFolder, entry: Arc<Queued>) {
        if entry.cancelled.load(Ordering::SeqCst) {
            return;
        }
        let cancel = CancellationToken::new();
        {
            let mut state = self.state.lock().unwrap();
            state.scanning.insert(
                folder.id.clone(),
                Scanning {
                    done: 0,
                    total: 0,
                    cancel: cancel.clone(),
                },
            );
            state.problems.remove(&folder.id);
        }
        self.changed();
        let progress_inner = Arc::downgrade(&self);
        let id = folder.id.clone();
        let options = ScanOptions {
            cache_dir: self.cache_dir.clone(),
            probe: self.probe.clone(),
            on_progress: Some(Arc::new(move |done, total| {
                let Some(inner) = progress_inner.upgrade() else {
                    return;
                };
                if let Some(scan) = inner.state.lock().unwrap().scanning.get_mut(&id) {
                    scan.done = done as u32;
                    scan.total = total as u32;
                }
                inner.progressed();
            })),
            cancel: Some(cancel.clone()),
        };
        let result = scan_folder(&folder, &options).await;
        {
            let mut state = self.state.lock().unwrap();
            match result {
                Ok(cache) => {
                    state
                        .caches
                        .insert(folder.id.clone(), Some(Arc::new(cache)));
                }
                Err(error) if !cancel.is_cancelled() => {
                    let missing = error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound);
                    let problem = if missing {
                        format!("{} isn't there. Is the drive connected?", folder.path)
                    } else {
                        error.to_string()
                    };
                    state.problems.insert(folder.id.clone(), problem);
                }
                Err(_) => {}
            }
            state.scanning.remove(&folder.id);
            if state
                .queued
                .get(&folder.id)
                .is_some_and(|queued| Arc::ptr_eq(queued, &entry))
            {
                state.queued.remove(&folder.id);
            }
        }
        self.changed();
    }
}

impl Library {
    pub fn new(options: LibraryOptions) -> Self {
        Self {
            inner: Arc::new(Inner {
                store: options.store,
                cache_dir: options.cache_dir,
                probe: options.probe,
                on_error: options
                    .on_error
                    .unwrap_or_else(|| Arc::new(|error| tracing::error!("{error:#}"))),
                state: Mutex::default(),
                listeners: Mutex::default(),
                queue: Serial::default(),
            }),
        }
    }

    /// Called after every change, and at most every 400 ms while a scan runs.
    pub fn on_change(&self, listener: impl Fn() + Send + Sync + 'static) {
        self.inner
            .listeners
            .lock()
            .unwrap()
            .push(Arc::new(listener));
    }

    /// The universal library's folders.
    pub fn folders(&self) -> Vec<LibraryFolder> {
        self.inner.store.folders()
    }

    pub fn save_folders(&self, folders: Vec<LibraryFolder>) {
        self.inner.store.save_folders(folders);
    }

    pub fn status(&self, folder: &LibraryFolder) -> LibraryFolderStatus {
        let mut state = self.inner.state.lock().unwrap();
        let cache = self.inner.cache_of(&mut state, folder);
        let scanning = state.scanning.get(&folder.id).map(|scan| ScanProgress {
            done: scan.done,
            total: scan.total,
        });
        let problem = state.problems.get(&folder.id).cloned().or_else(|| {
            cache.as_ref().filter(|cache| cache.truncated).map(|_| {
                "Only the first 20,000 files are listed. Add the folders inside it instead."
                    .to_string()
            })
        });
        LibraryFolderStatus {
            folder: folder.clone(),
            files: cache.as_ref().map_or(0, |cache| cache.files.len() as u32),
            scanning,
            scanned_at: cache.map(|cache| cache.scanned_at.clone()),
            problem,
        }
    }

    /// Every file in a chain of libraries, with what the user noted on it.
    pub fn files(&self, chain: &[LibraryLink]) -> Vec<LibraryFile> {
        let mut files = Vec::new();
        for link in chain {
            for folder in &link.folders {
                let cache = {
                    let mut state = self.inner.state.lock().unwrap();
                    self.inner.cache_of(&mut state, folder)
                };
                for scanned in cache.iter().flat_map(|cache| &cache.files) {
                    let path = in_folder(folder, &scanned.rel);
                    let data = self.inner.store.data(&path.to_string_lossy());
                    files.push(Inner::file_of(link, folder, scanned, data));
                }
            }
        }
        files
    }

    /// The library file at an absolute path in this chain, most specific library first; None when it isn't one.
    pub fn find(&self, chain: &[LibraryLink], file: &str) -> Option<LibraryFile> {
        let key = path_key(file);
        for link in chain.iter().rev() {
            for folder in &link.folders {
                if !is_inside(path_key(&folder.path), &key) {
                    continue;
                }
                let cache = {
                    let mut state = self.inner.state.lock().unwrap();
                    self.inner.cache_of(&mut state, folder)
                };
                let scanned = cache.as_ref().and_then(|cache| {
                    cache
                        .files
                        .iter()
                        .find(|entry| path_key(in_folder(folder, &entry.rel)) == key)
                });
                if let Some(scanned) = scanned {
                    return Some(Inner::file_of(
                        link,
                        folder,
                        scanned,
                        self.inner.store.data(file),
                    ));
                }
            }
        }
        None
    }

    /// The files that match, from the most specific library first.
    pub fn search(&self, chain: &[LibraryLink], query: &LibraryQuery) -> Vec<LibraryFile> {
        let order: Vec<_> = chain.iter().map(|link| scope_kind(&link.scope)).collect();
        search_library(self.files(chain), query, &order)
    }

    /// Scans a folder after the ones already waiting; a folder already waiting isn't queued twice. It's queued
    /// when called; the future only waits for it.
    pub fn scan(&self, folder: &LibraryFolder) -> impl Future<Output = ()> + Send + 'static {
        let entry = {
            let mut state = self.inner.state.lock().unwrap();
            match state.queued.get(&folder.id) {
                Some(pending)
                    if !pending.cancelled.load(Ordering::SeqCst) && pending.path == folder.path =>
                {
                    pending.clone()
                }
                _ => {
                    let (done, receiver) = watch::channel(false);
                    let entry = Arc::new(Queued {
                        path: folder.path.clone(),
                        cancelled: AtomicBool::new(false),
                        done: receiver,
                    });
                    state.queued.insert(folder.id.clone(), entry.clone());
                    let job = self.inner.clone().run_scan(folder.clone(), entry.clone());
                    self.inner.queue.spawn(async move {
                        job.await;
                        done.send_replace(true);
                    });
                    entry
                }
            }
        };
        let mut done = entry.done.clone();
        async move {
            let _ = done.wait_for(|done| *done).await;
        }
    }

    /// Rescans every folder, measuring only what changed: at start, so files added meanwhile show up.
    pub fn scan_all(&self, folders: &[LibraryFolder]) -> impl Future<Output = ()> + Send + 'static {
        let scans: Vec<_> = folders.iter().map(|folder| self.scan(folder)).collect();
        async move {
            for scan in scans {
                scan.await;
            }
        }
    }

    /// Forgets a folder that left every library: its scan stops and its cache goes. Its files stay.
    pub fn forget(&self, folder: &LibraryFolder) {
        {
            let mut state = self.inner.state.lock().unwrap();
            if let Some(pending) = state.queued.get(&folder.id) {
                pending.cancelled.store(true, Ordering::SeqCst);
            }
            if let Some(scan) = state.scanning.get(&folder.id) {
                scan.cancel.cancel();
            }
            state.caches.remove(&folder.id);
            state.problems.remove(&folder.id);
        }
        if let Err(error) = remove_cache(&self.inner.cache_dir, &folder.id) {
            (self.inner.on_error)(error.into());
        }
        self.inner.changed();
    }

    pub fn update_file(&self, file: &str, change: FileChange) {
        self.inner.store.update_data(file, &|data| FileData {
            tags: change.tags.clone().unwrap_or(data.tags),
            star: change.star.unwrap_or(data.star),
            note: change.note.clone().unwrap_or(data.note),
            used_in: data.used_in,
        });
        self.inner.changed();
    }

    /// Notes that an agent brought a file into a project's timeline; the same project and timeline once.
    pub fn record_use(&self, file: &str, used: LibraryUse) {
        self.inner.store.update_data(file, &|mut data| {
            data.used_in.retain(|other| {
                !(other.project_id == used.project_id && other.timeline == used.timeline)
            });
            data.used_in.push(used.clone());
            data
        });
        self.inner.changed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::folders::memory_library_store;
    use crate::library::testing::{counting_probe, folder, temp, write};

    #[tokio::test]
    async fn finds_a_file_by_its_path_in_the_most_specific_library_with_whats_noted_on_it() {
        let shared = temp();
        let own = temp();
        let cache = temp();
        write(&shared.path().join("Song.wav"), "x");
        write(&own.path().join("Theme.wav"), "x");
        let shared_path = shared.path().to_string_lossy().into_owned();
        let own_path = own.path().to_string_lossy().into_owned();
        let library = Library::new(LibraryOptions {
            store: Arc::new(memory_library_store(vec![folder(
                &shared_path,
                "music",
                "shared",
            )])),
            cache_dir: cache.path().to_path_buf(),
            probe: counting_probe(),
            on_error: None,
        });
        let own_folder = LibraryFolder {
            label: "Ours".into(),
            ..folder(&own_path, "music", "own")
        };
        let mut all = library.folders();
        all.push(own_folder.clone());
        library.scan_all(&all).await;
        let chain = vec![
            LibraryLink {
                scope: LibraryScope::Universal,
                label: "Universal".into(),
                folders: library.folders(),
            },
            LibraryLink {
                scope: LibraryScope::Project {
                    project_id: "p1".into(),
                },
                label: "Doc".into(),
                folders: vec![own_folder],
            },
        ];
        let song = shared
            .path()
            .join("Song.wav")
            .to_string_lossy()
            .into_owned();
        let theme = own.path().join("Theme.wav").to_string_lossy().into_owned();
        library.update_file(
            &song,
            FileChange {
                star: Some(true),
                ..FileChange::default()
            },
        );
        let found = library.find(&chain, &song).unwrap();
        assert_eq!(
            (found.scope, found.star, found.rel.as_str()),
            (LibraryScopeKind::Universal, true, "Song.wav")
        );
        let found = library.find(&chain, &theme).unwrap();
        assert_eq!(
            (found.scope, found.folder_id.as_str()),
            (LibraryScopeKind::Project, "own")
        );
        let nope = own.path().join("Nope.wav").to_string_lossy().into_owned();
        assert_eq!(library.find(&chain, &nope), None);
        assert_eq!(library.find(&chain[..1], &theme), None);
        let names: Vec<_> = library
            .search(
                &chain,
                &LibraryQuery {
                    kind: Some(linker_edit_protocol::LibraryKind::Music),
                    ..LibraryQuery::default()
                },
            )
            .into_iter()
            .map(|file| file.name)
            .collect();
        assert_eq!(names, ["Theme.wav", "Song.wav"]);
    }

    #[tokio::test]
    async fn says_when_a_folder_is_gone_and_forgets_one_that_left() {
        let dir = temp();
        let cache = temp();
        write(&dir.path().join("Song.wav"), "x");
        let probe = counting_probe();
        let library = Library::new(LibraryOptions {
            store: Arc::new(memory_library_store(vec![])),
            cache_dir: cache.path().to_path_buf(),
            probe: probe.clone(),
            on_error: None,
        });
        let changes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = changes.clone();
        library.on_change(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        let gone = folder(&dir.path().join("gone").to_string_lossy(), "music", "gone");
        library.scan(&gone).await;
        assert_eq!(
            library.status(&gone).problem,
            Some(format!(
                "{} isn't there. Is the drive connected?",
                gone.path
            ))
        );
        let music = folder(&dir.path().to_string_lossy(), "music", "music");
        // Asked twice while waiting: scanned once.
        let (first, second) = (library.scan(&music), library.scan(&music));
        first.await;
        second.await;
        assert_eq!(probe.calls_sorted(), ["Song.wav"]);
        let status = library.status(&music);
        assert_eq!(
            (status.files, status.scanning, status.problem),
            (1, None, None)
        );
        assert!(changes.load(Ordering::SeqCst) >= 4);
        assert!(super::super::scan::cache_file(cache.path(), "music").exists());
        library.forget(&music);
        assert_eq!(library.status(&music).files, 0);
        assert!(!super::super::scan::cache_file(cache.path(), "music").exists());
        assert!(dir.path().join("Song.wav").exists());
    }
}
