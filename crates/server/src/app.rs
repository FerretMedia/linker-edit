// All app state lives here; clients get a snapshot of the thread they look at, and patches after. The port of
// server/app.ts.
//
// The state sits behind one lock, and every change happens with it held, as one synchronous run of the
// TypeScript's event loop did: handle_message changes what it can at once and spawns what waits (on Resolve, an
// agent, a history save), and those come back for the lock when they're done. A service that tells the app about a
// change from inside a call the app made with the lock held (a status listener, a preview's stop) gets its turn
// when the lock is let go, before handle_message returns; see App::run_locked.

use std::future::Future;
use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, Weak};

use futures::FutureExt;
use futures::future::Shared;
use indexmap::IndexMap;
use linker_edit_protocol::{
    AppMeta, AppState, ApprovalMode, Blocked, ChatItem, ClientMessage, CollectionMessage,
    ComputerSetup, LibraryClientMessage, LibraryFile, LibraryFolder, LibraryQuery, LibraryScope,
    LibraryScopeOption, LibraryUse, LibraryView, LinkerAccount, LinkerConnect, LinkerConnectState,
    LocalLink, OrError, PlayableTimeline, PreviewState, PreviewTimeline, ProjectSidebarChanges,
    ProjectSidebarState, ProjectSource, ProjectSummary, Provider, RemoteAccess, ResolveLink,
    ResolveLinkKind, ResolveStatus, ServerMessage, SidebarClientMessage, TimelineRef, Tone,
    TranscriptionStatus,
};
use serde::Serialize;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use crate::app_update::AppUpdates;
use crate::collections::collections::{Collections, CollectionsHost};
use crate::collections::store::CollectionStore;
use crate::collections::testing::memory_collection_store;
use crate::collections::tools::{SAVE_BRIEF, SAVE_EXAMPLE, is_brief_tool};
use crate::connect::connect::Connect;
use crate::folders::{browse_folders, expand_home};
use crate::history::history::{History, MediaFile, OpenSnapshotProject, ProjectCheck};
use crate::js;
use crate::library::folders::{
    check_folder, clean_folder, clean_tags, keep_library, new_folder, project_folders,
    with_project_folders,
};
use crate::library::serve::{ServedFile, library_file};
use crate::library::tools::{
    ImportContext, LIBRARY_IMPORT, LIBRARY_SEARCH, describe_file, library_import, query_from_input,
};
use crate::library::{FileChange, Library, LibraryLink};
use crate::linker::Linker;
use crate::preview::preview::{PreviewEvent, PreviewJob, PreviewRenderer};
use crate::projects::link::{
    Linked, OpenProject, is_linked_project, link_from_open, link_kind, open_project, refreshed_link,
};
use crate::projects::store::{
    NEW_THREAD_SIDEBAR, NewProject, ProjectCollection, ProjectStore, SavedProject, SavedThread,
};
use crate::providers::claude::ApprovalModeFn;
use crate::providers::types::ProviderSession;
use crate::remote::RemoteAccessService;
use crate::resolve::tools::{ResolveToolKind, find_resolve_tool};
use crate::resolve::versions::{Applied, read_origins};
use crate::resolve_projects::ResolveProjects;
use crate::rules::{LINKER_MCP_NAME, PROJECT_NOTES_FILE, read_project_file, write_project_file};
use crate::settings::{Settings, SettingsStore};
use crate::study::{StudyService, is_study_tool};
use crate::thread::{DEFAULT_TITLE, Events, Thread, ThreadEvent, provider_name, run_turn};
use crate::transcribe::transcriber::DESCRIBE_BUDGET;
use crate::transcribe::{Transcriber, describe_results};

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// What createSession was given: resume_id continues a conversation from before a restart; dir is the project's
/// working folder. rules gives the agent's rules for the project as they are when a turn starts.
pub struct SessionRequest {
    pub provider: Provider,
    pub approval_mode: ApprovalModeFn,
    pub dir: PathBuf,
    pub resume_id: Option<String>,
    pub rules: Arc<dyn Fn() -> String + Send + Sync>,
}

pub type CreateSession = Arc<dyn Fn(SessionRequest) -> Arc<dyn ProviderSession> + Send + Sync>;
pub type CheckResolve = Arc<dyn Fn() -> BoxFuture<ResolveStatus> + Send + Sync>;
pub type CreateHistory = Arc<dyn Fn(&Path, ProjectCheck) -> Arc<dyn History> + Send + Sync>;
pub type PrepareFolder = Arc<dyn Fn(&Path, &str) -> anyhow::Result<()> + Send + Sync>;
/// Runs one of Linker-Edit's own Resolve tools for an agent working in the project folder dir.
pub type ResolveTools = Arc<
    dyn Fn(
            String,
            Value,
            PathBuf,
            Option<CancellationToken>,
        ) -> BoxFuture<anyhow::Result<ToolOutput>>
        + Send
        + Sync,
>;
/// Backs the open Resolve project up into the folder; where to.
pub type BackupProject = Arc<dyn Fn(PathBuf) -> BoxFuture<anyhow::Result<String>> + Send + Sync>;
/// Makes an approved AI timeline the main one, keeping the old one under another name.
pub type ApplyTimeline =
    Arc<dyn Fn(PathBuf, String) -> BoxFuture<anyhow::Result<Applied>> + Send + Sync>;

/// What a Resolve tool gives the agent: its text, and images to see.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolOutput {
    pub text: String,
    pub images: Option<Vec<String>>,
}

/// An agent's tool call answered: `{ text, images?, isError }`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolReply {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
    #[serde(rename = "isError")]
    pub is_error: bool,
}

impl ToolReply {
    pub fn ok(text: impl Into<String>) -> Self {
        ToolReply {
            text: text.into(),
            images: None,
            is_error: false,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        ToolReply {
            text: text.into(),
            images: None,
            is_error: true,
        }
    }
}

pub struct AppOptions {
    pub version: String,
    pub store: Arc<dyn ProjectStore>,
    pub create_session: CreateSession,
    pub check_resolve: CheckResolve,
    /// Each project's edit history in its folder; saving is off without it.
    pub create_history: Option<CreateHistory>,
    /// Renders timelines through Resolve for the preview panel.
    pub preview: Option<Arc<dyn PreviewRenderer>>,
    /// Lists and opens Resolve projects; without it projects can only follow what Resolve has open.
    pub resolve_projects: Option<Arc<dyn ResolveProjects>>,
    /// Gets a project folder ready for agents, such as its AGENTS.md with the project's rules.
    pub prepare_folder: Option<PrepareFolder>,
    /// Workspaces and series; kept in memory only without it.
    pub collections: Option<Arc<dyn CollectionStore>>,
    pub resolve_tools: Option<ResolveTools>,
    /// Agents change nothing in Resolve without it.
    pub backup_project: Option<BackupProject>,
    pub apply_timeline: Option<ApplyTimeline>,
    /// Speech to text for the agents' transcribe tool.
    pub transcriber: Option<Arc<Transcriber>>,
    /// Opening the app from the user's other devices over Tailscale.
    pub remote: Option<Arc<RemoteAccessService>>,
    /// The optional Linker account.
    pub linker: Option<Arc<Linker>>,
    /// Linker Connect: this computer at its own address at Linker, for your other computers.
    pub connect: Option<Arc<Connect>>,
    /// Choices kept across restarts, such as supervised mode.
    pub settings: Option<Arc<dyn SettingsStore>>,
    /// Where this computer keeps things, shown on the settings page.
    pub setup: Option<ComputerSetup>,
    /// The updates of the desktop app this server runs in; none with `npx linker-edit`.
    pub app_updates: Option<Arc<AppUpdates>>,
    /// Folders of files the user reuses, for the library page and the agents' library tools.
    pub library: Option<Arc<Library>>,
    /// Measures example videos and compares a cut with its raw footage, for the agents' study tools.
    pub study: Option<Arc<StudyService>>,
}

impl AppOptions {
    /// What every app needs; the rest is off until set.
    pub fn new(
        version: impl Into<String>,
        store: Arc<dyn ProjectStore>,
        create_session: CreateSession,
        check_resolve: CheckResolve,
    ) -> Self {
        AppOptions {
            version: version.into(),
            store,
            create_session,
            check_resolve,
            create_history: None,
            preview: None,
            resolve_projects: None,
            prepare_folder: None,
            collections: None,
            resolve_tools: None,
            backup_project: None,
            apply_timeline: None,
            transcriber: None,
            remote: None,
            linker: None,
            connect: None,
            settings: None,
            setup: None,
            app_updates: None,
            library: None,
            study: None,
        }
    }
}

/// Starts the chat line that marks an approved edit; feedback rounds are counted from the last one.
const APPROVED: &str = "Approved:";
/// Starts the message Linker-Edit sends for the user after an approval, so it isn't counted as feedback.
const LEARN: &str = "Learn from this edit:";
/// The Resolve tools that work on files alone, which a setup thread may use.
fn uses_no_resolve(tool: &str, input: &Value) -> bool {
    tool == "find_silences"
        || (tool == "view_frames" && input.get("file").is_some_and(Value::is_string))
}
const SETUP_REFUSAL: &str = "This is a setup thread: it has no Resolve project, so tools that work in Resolve are refused here. Study the videos as files instead.";

/// How many matching files the library page gets at once.
const LIBRARY_PAGE: usize = 300;

/// Linker Connect in builds and tests without it.
fn no_connect() -> LinkerConnect {
    LinkerConnect {
        state: LinkerConnectState::SignedOut,
        url: None,
        connected: false,
        step: None,
        problem: None,
        computer_id: String::new(),
        computers: None,
        limit: 3,
        at_limit: false,
        devices: Vec::new(),
    }
}

/// encodeURIComponent's set: everything but letters, digits and - _ . ! ~ * ' ( ).
const URI_COMPONENT: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

pub fn encode_uri_component(text: &str) -> String {
    percent_encoding::utf8_percent_encode(text, URI_COMPONENT).to_string()
}

/// One client: a browser tab, the window, the iPad. send gets each ServerMessage as JSON; it must not block.
pub trait Client: Send + Sync {
    fn send(&self, data: String);
}

pub type ClientId = u64;

struct ProjectState {
    saved: SavedProject,
    history: Option<Arc<dyn History>>,
}

struct ThreadEntry {
    thread: Thread,
    project_id: String,
    context: Arc<TurnContext>,
}

struct ClientEntry {
    client: Arc<dyn Client>,
    thread_id: Option<String>,
}

#[derive(Clone)]
struct LibraryWanted {
    scope: LibraryScope,
    query: LibraryQuery,
}

struct Hub {
    resolve: ResolveStatus,
    resolve_here: bool,
    preview: PreviewState,
    // The project the preview is of: other projects' threads see none. Resolve renders one at a time.
    preview_project: Option<String>,
    preview_job: Option<PreviewJob>,
    update: Option<String>,
    // Resolve has one project open and does one thing at a time: one turn, open or render app-wide.
    turn_owner: Option<String>,
    opening: Option<String>,
    // The project whose AI timeline is becoming its main one.
    applying: Option<String>,
    // Agents' Resolve tool calls still running. A stopped turn ends before a render it started has wound
    // down, and Resolve stays taken until it has.
    tool_calls: u32,
    // New threads start with the agent picked last.
    last_provider: Provider,
    projects: IndexMap<String, ProjectState>,
    threads: IndexMap<String, ThreadEntry>,
    clients: IndexMap<ClientId, ClientEntry>,
    // What each client's library page shows, so it's sent again whenever the library changes.
    library_views: IndexMap<ClientId, LibraryWanted>,
    resolve_check: Option<Shared<BoxFuture<()>>>,
}

struct State {
    hub: Hub,
    collections: Collections,
}

type Pending = Box<dyn for<'a> FnOnce(&mut Locked<'a>) + Send>;

struct Inner {
    options: AppOptions,
    state: Mutex<State>,
    // The thread holding the lock, 0 when none: a call back from inside a locked section waits in pending.
    owner: AtomicU64,
    pending: Mutex<Vec<Pending>>,
    approval_mode: Arc<RwLock<ApprovalMode>>,
    next_client: AtomicU64,
    me: Weak<Inner>,
}

/// The app: cheap to clone, every clone the same app.
#[derive(Clone)]
pub struct App {
    inner: Arc<Inner>,
}

static NEXT_THREAD: AtomicU64 = AtomicU64::new(1);
thread_local! {
    static THREAD_ID: u64 = NEXT_THREAD.fetch_add(1, Ordering::SeqCst);
}

fn current_thread() -> u64 {
    THREAD_ID.with(|id| *id)
}

fn error_text(error: &anyhow::Error) -> String {
    error.to_string()
}

/// Commits the project's linker-edit.json in its edit history, in the background.
fn commit_project_file(history: Option<Arc<dyn History>>, message: String) {
    let Some(history) = history else {
        return;
    };
    let mut commit: BoxFuture<()> = Box::pin(async move {
        if let Err(error) = history.commit_project_file(&message).await {
            tracing::error!("Couldn't commit linker-edit.json: {error:#}");
        }
    });
    // Started at once, as the TypeScript's promise was; what it then waits for finishes in the background.
    if (&mut commit).now_or_never().is_none() {
        tokio::spawn(commit);
    }
}

/// The app's state while its lock is held. Calls that came back from inside it run when it's let go.
struct Locked<'a> {
    app: &'a App,
    guard: ManuallyDrop<MutexGuard<'a, State>>,
}

impl Drop for Locked<'_> {
    fn drop(&mut self) {
        loop {
            let next = {
                let mut pending = self.app.inner.pending.lock().unwrap();
                (!pending.is_empty()).then(|| pending.remove(0))
            };
            match next {
                Some(run) => run(self),
                None => break,
            }
        }
        self.app.inner.owner.store(0, Ordering::SeqCst);
        // SAFETY: dropped once, here, and never used after.
        unsafe { ManuallyDrop::drop(&mut self.guard) }
    }
}

/// The workspaces' and series' view of the app: its projects, and what they change in them.
struct CollectionHost<'a> {
    hub: &'a mut Hub,
    store: &'a dyn ProjectStore,
    removed: Vec<String>,
    changed: bool,
}

impl CollectionsHost for CollectionHost<'_> {
    fn projects(&self) -> Vec<SavedProject> {
        self.hub
            .projects
            .values()
            .map(|project| project.saved.clone())
            .collect()
    }

    fn set_collection(
        &mut self,
        project_id: &str,
        collection: Option<ProjectCollection>,
        commit_message: &str,
    ) {
        let Some(project) = self.hub.projects.get_mut(project_id) else {
            return;
        };
        project.saved.collection = collection;
        self.store.save_project(&project.saved);
        commit_project_file(project.history.clone(), commit_message.to_string());
    }

    fn remove_project(&mut self, project_id: &str) {
        self.removed.push(project_id.to_string());
    }

    fn changed(&mut self) {
        self.changed = true;
    }
}

/// For summaries, which only read the projects.
struct ReadHost<'a>(&'a Hub);

impl CollectionsHost for ReadHost<'_> {
    fn projects(&self) -> Vec<SavedProject> {
        self.0
            .projects
            .values()
            .map(|project| project.saved.clone())
            .collect()
    }
    fn set_collection(&mut self, _: &str, _: Option<ProjectCollection>, _: &str) {}
    fn remove_project(&mut self, _: &str) {}
    fn changed(&mut self) {}
}

/// A turn of one thread: the TypeScript's ThreadHost, bound to the thread and its project.
pub(crate) struct TurnContext {
    app: Weak<Inner>,
    thread_id: String,
    project_id: String,
    dir: PathBuf,
    history: Option<Arc<dyn History>>,
}

impl TurnContext {
    fn app(&self) -> Option<App> {
        self.app.upgrade().map(|inner| App { inner })
    }

    pub(crate) fn thread_id(&self) -> &str {
        &self.thread_id
    }

    pub(crate) fn history(&self) -> Option<Arc<dyn History>> {
        self.history.clone()
    }

    /// Runs f on the thread when it is still there (not deleted), and carries out what it changed.
    pub(crate) fn with_thread<R>(
        &self,
        f: impl FnOnce(&mut Thread, &mut Events) -> R,
    ) -> Option<R> {
        let app = self.app()?;
        let mut locked = app.lock();
        locked.with_thread(&self.thread_id, f)
    }

    /// Err is why this thread can't run against Resolve now; Ok a notice when it can.
    pub(crate) async fn check(&self) -> Result<Option<String>, String> {
        match self.app() {
            Some(app) => app.check_before_turn(&self.project_id).await,
            None => Ok(None),
        }
    }

    pub(crate) fn release(&self) {
        let Some(app) = self.app() else { return };
        let mut locked = app.lock();
        if locked.guard.hub.turn_owner.as_deref() == Some(self.thread_id.as_str()) {
            locked.guard.hub.turn_owner = None;
        }
        // The agent may have written PROJECT.md in its turn.
        locked.publish_project_file(&self.project_id, None);
    }

    pub(crate) fn changed(&self) {
        if let Some(app) = self.app() {
            app.lock().send_meta();
        }
    }

    pub(crate) fn resolve_connected(&self) -> bool {
        self.app().is_some_and(|app| {
            matches!(
                app.lock().guard.hub.resolve,
                ResolveStatus::Connected { .. }
            )
        })
    }

    pub(crate) fn shared(&self) -> bool {
        self.app().is_some_and(|app| {
            app.lock()
                .guard
                .hub
                .projects
                .get(&self.project_id)
                .and_then(|project| project.saved.link.as_ref())
                .is_some_and(|link| link.kind == ResolveLinkKind::Cloud)
        })
    }

    pub(crate) async fn saved(&self) {
        let Some(app) = self.app() else { return };
        {
            let mut locked = app.lock();
            let hub = &mut locked.guard.hub;
            if let PreviewState::Ready { outdated, .. } = &mut hub.preview
                && hub.preview_project.as_deref() == Some(self.project_id.as_str())
            {
                *outdated = true;
            }
        }
        app.publish_history(&self.project_id, None).await;
    }

    pub(crate) async fn backup(&self) -> anyhow::Result<Option<String>> {
        let Some(backup) = self
            .app()
            .and_then(|app| app.inner.options.backup_project.clone())
        else {
            return Ok(None);
        };
        backup(self.dir.clone()).await.map(Some)
    }

    pub(crate) fn create_session(&self, provider: Provider) -> Arc<dyn ProviderSession> {
        let app = self.app().expect("the app outlives its turns");
        let saved = {
            let locked = app.lock();
            locked
                .guard
                .hub
                .projects
                .get(&self.project_id)
                .map(|project| project.saved.clone())
        };
        let saved = saved.unwrap_or_else(|| SavedProject {
            id: self.project_id.clone(),
            dir: self.dir.clone(),
            name: None,
            link: None,
            local: LocalLink::default(),
            icon: None,
            pinned: false,
            archived: false,
            collection: None,
            setup: false,
        });
        app.session_for(&saved, provider, None)
    }

    pub(crate) fn refresh_resolve(&self) {
        if let Some(app) = self.app() {
            drop(app.refresh_resolve());
        }
    }
}

/// An approval swap under way: what apply_timeline needs once Resolve was taken for it.
struct Applying {
    apply: ApplyTimeline,
    dir: PathBuf,
    history: Option<Arc<dyn History>>,
    learner: Option<String>,
}

/// The thread whose turn may use Resolve now.
struct RunningTurn {
    thread_id: String,
    project_id: String,
    dir: PathBuf,
    signal: Option<CancellationToken>,
}

pub fn create_app(options: AppOptions) -> App {
    let settings = options
        .settings
        .as_ref()
        .map(|settings| settings.load())
        .unwrap_or_default();
    let collections = Collections::new(
        options
            .collections
            .clone()
            .unwrap_or_else(|| Arc::new(memory_collection_store(Vec::new()))),
    );
    let inner = Arc::new_cyclic(|me| Inner {
        state: Mutex::new(State {
            hub: Hub {
                resolve: ResolveStatus::Checking,
                resolve_here: settings.resolve_here.unwrap_or(true),
                preview: PreviewState::Idle,
                preview_project: None,
                preview_job: None,
                update: None,
                turn_owner: None,
                opening: None,
                applying: None,
                tool_calls: 0,
                last_provider: settings.default_provider.unwrap_or(Provider::Claude),
                projects: IndexMap::new(),
                threads: IndexMap::new(),
                clients: IndexMap::new(),
                library_views: IndexMap::new(),
                resolve_check: None,
            },
            collections,
        }),
        options,
        owner: AtomicU64::new(0),
        pending: Mutex::default(),
        // Full access by default, like T3 Code: the rules are enforced in code, and the edit history undoes.
        // A switch to supervised is kept, so a restart never quietly turns it back into full access.
        approval_mode: Arc::new(RwLock::new(
            settings.approval_mode.unwrap_or(ApprovalMode::Auto),
        )),
        next_client: AtomicU64::new(1),
        me: me.clone(),
    });
    let app = App { inner };
    {
        let mut locked = app.lock();
        for loaded in app.inner.options.store.load() {
            locked.add_project(loaded.project, loaded.threads);
        }
    }
    let options = &app.inner.options;
    let meta = {
        let me = app.inner.me.clone();
        move || {
            if let Some(inner) = me.upgrade() {
                App { inner }.run_locked(|locked| locked.send_meta());
            }
        }
    };
    if let Some(transcriber) = &options.transcriber {
        let meta = meta.clone();
        let _keep = transcriber.on_status(move |_| meta());
    }
    if let Some(remote) = &options.remote {
        remote.on_status(Arc::new(meta.clone()));
    }
    if let Some(linker) = &options.linker {
        linker.on_status(Arc::new(meta.clone()));
    }
    if let Some(connect) = &options.connect {
        connect.on_status(Arc::new(meta.clone()));
    }
    if let Some(updates) = &options.app_updates {
        updates.on_status(Arc::new(meta.clone()));
    }
    if let Some(library) = &options.library {
        let me = app.inner.me.clone();
        library.on_change(move || {
            if let Some(inner) = me.upgrade() {
                App { inner }.run_locked(|locked| locked.send_library_views());
            }
        });
    }
    app
}

impl App {
    fn lock(&self) -> Locked<'_> {
        let guard = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.inner.owner.store(current_thread(), Ordering::SeqCst);
        Locked {
            app: self,
            guard: ManuallyDrop::new(guard),
        }
    }

    /// Runs f with the lock; when this thread holds it already (a service calling back from inside a locked
    /// call), right after that section, before the lock is let go.
    fn run_locked(&self, f: impl for<'a> FnOnce(&mut Locked<'a>) + Send + 'static) {
        if self.inner.owner.load(Ordering::SeqCst) == current_thread() {
            self.inner.pending.lock().unwrap().push(Box::new(f));
        } else {
            f(&mut self.lock());
        }
    }

    pub fn version(&self) -> &str {
        &self.inner.options.version
    }

    fn approval_mode(&self) -> ApprovalMode {
        *self.inner.approval_mode.read().unwrap()
    }

    fn session_for(
        &self,
        saved: &SavedProject,
        provider: Provider,
        resume_id: Option<String>,
    ) -> Arc<dyn ProviderSession> {
        let mode = self.inner.approval_mode.clone();
        let me = self.inner.me.clone();
        let project = saved.clone();
        let rules = Arc::new(move || {
            let Some(inner) = me.upgrade() else {
                return String::new();
            };
            let app = App { inner };
            let locked = app.lock();
            let saved = locked
                .guard
                .hub
                .projects
                .get(&project.id)
                .map_or(&project, |current| &current.saved);
            locked.guard.collections.rules_for(saved)
        });
        (self.inner.options.create_session)(SessionRequest {
            provider,
            approval_mode: Arc::new(move || *mode.read().unwrap()),
            dir: saved.dir.clone(),
            resume_id,
            rules,
        })
    }

    /// A client connected: it lands on a thread and gets a snapshot of it. disconnect when it goes.
    pub fn connect(&self, client: Arc<dyn Client>) -> ClientId {
        let id = self.inner.next_client.fetch_add(1, Ordering::SeqCst);
        let mut locked = self.lock();
        let thread_id = locked.default_thread(None);
        locked
            .guard
            .hub
            .clients
            .insert(id, ClientEntry { client, thread_id });
        locked.send_snapshot(id);
        id
    }

    pub fn disconnect(&self, client: ClientId) {
        let mut locked = self.lock();
        locked.guard.hub.clients.shift_remove(&client);
        locked.guard.hub.library_views.shift_remove(&client);
    }

    pub fn handle_message(&self, client: ClientId, message: ClientMessage) {
        let app = self.clone();
        let mut locked = self.lock();
        locked.handle_message(&app, client, message);
    }

    /// Asks Resolve how it is; one check at a time, which every caller waits for. Starts when called.
    pub fn refresh_resolve(&self) -> impl Future<Output = ()> + Send + 'static {
        let mut locked = self.lock();
        if let Some(check) = &locked.guard.hub.resolve_check {
            return check.clone();
        }
        let app = self.clone();
        let check: Shared<BoxFuture<()>> = (Box::pin(async move {
            let status = (app.inner.options.check_resolve)().await;
            let mut locked = app.lock();
            locked.apply_resolve_status(status);
            locked.guard.hub.resolve_check = None;
        }) as BoxFuture<()>)
            .shared();
        locked.guard.hub.resolve_check = Some(check.clone());
        drop(locked);
        tokio::spawn(check.clone());
        check
    }

    /// Downloads the speech engine in the background, so nobody has to ask for it and an agent's first
    /// transcription doesn't wait on it. Not on a computer without Resolve: agents don't work there.
    pub fn prepare_transcription(&self) {
        self.lock().prepare_transcription();
    }

    /// A newer release the UI offers, or None.
    pub fn set_update(&self, version: Option<String>) {
        let mut locked = self.lock();
        if locked.guard.hub.update == version {
            return;
        }
        locked.guard.hub.update = version;
        locked.send_meta();
    }

    pub fn media_file(&self, id: &str) -> Option<MediaFile> {
        let histories: Vec<Arc<dyn History>> = self
            .lock()
            .guard
            .hub
            .projects
            .values()
            .filter_map(|project| project.history.clone())
            .collect();
        histories.iter().find_map(|history| history.media_file(id))
    }

    pub fn preview_file(&self, name: &str) -> Option<PathBuf> {
        self.inner.options.preview.as_ref()?.file(name)
    }

    pub async fn playable_timeline(
        &self,
        project_id: Option<&str>,
        id: Option<String>,
        commit: Option<String>,
    ) -> anyhow::Result<Option<PlayableTimeline>> {
        let (history, dir) = {
            let locked = self.lock();
            let project = project_id.and_then(|id| locked.guard.hub.projects.get(id));
            let Some((history, project)) =
                project.and_then(|project| Some((project.history.clone()?, project)))
            else {
                anyhow::bail!("This project has no edit history, so there is nothing to play.");
            };
            // The live timeline comes from the project open in Resolve, which must be this one.
            if commit.is_none()
                && let Some(refusal) = locked.project_refusal(&project.saved.id)
            {
                anyhow::bail!(refusal);
            }
            (history, project.saved.dir.clone())
        };
        let live = commit.is_none();
        let Some(mut timeline) = history.timeline(id, commit).await? else {
            return Ok(None);
        };
        // As Resolve has it now, an AI timeline says which main timeline it can replace.
        if live && let Some(main) = read_origins(&dir).get(&timeline.id) {
            timeline.replaces = Some(TimelineRef {
                id: main.id.clone(),
                name: main.name.clone(),
            });
        }
        Ok(Some(timeline))
    }

    /// A turn, a render, an open or an approval swap is using Resolve.
    pub fn busy(&self) -> bool {
        self.lock().resolve_busy()
    }

    async fn check_before_turn(&self, project_id: &str) -> Result<Option<String>, String> {
        {
            let locked = self.lock();
            let Some(saved) = locked.saved(project_id) else {
                return Ok(None);
            };
            locked.prepare_folder(&saved);
            // Never linked to whatever Resolve has open: it studies files.
            if saved.setup {
                return Ok(None);
            }
        }
        self.refresh_resolve().await;
        let mut locked = self.lock();
        if !locked.guard.hub.projects.contains_key(project_id) {
            return Ok(None);
        }
        if let Some(refusal) = locked.project_refusal(project_id) {
            return Err(refusal);
        }
        let Some(open) = open_project(&locked.guard.hub.resolve) else {
            return Ok(None);
        };
        if locked.guard.hub.projects[project_id].saved.link.is_some() {
            return Ok(None);
        }
        locked.set_link(
            project_id,
            link_from_open(&open),
            Some(&format!("Link to Resolve project \"{}\"", open.name)),
        );
        let link = locked.guard.hub.projects[project_id]
            .saved
            .link
            .clone()
            .expect("just linked");
        Ok(Some(format!(
            "Linked this project to the Resolve project \"{}\" in {}.",
            open.name,
            describe_library(&link, &open)
        )))
    }

    async fn publish_history(&self, project_id: &str, client: Option<ClientId>) {
        let history = {
            let locked = self.lock();
            locked
                .guard
                .hub
                .projects
                .get(project_id)
                .and_then(|project| project.history.clone())
        };
        let Some(history) = history else { return };
        let entries = history.log(None).await.unwrap_or_default();
        let message = ServerMessage::History {
            project_id: project_id.to_string(),
            entries,
        };
        let locked = self.lock();
        match client {
            Some(client) => locked.send(client, &message),
            None => locked.send_all(&message),
        }
    }

    /// The user approved an AI timeline: the project is backed up, then it takes the main timeline's name and
    /// place, and the main one stays, renamed. The swap lands in the edit history. Locked::begin_apply started it.
    async fn apply_timeline(
        &self,
        client: ClientId,
        project_id: String,
        timeline_id: String,
        applying: Applying,
    ) {
        let Applying {
            apply,
            dir,
            history,
            learner,
        } = applying;
        let result: anyhow::Result<Option<String>> = async {
            self.refresh_resolve().await;
            if let Some(refusal) = self.lock().project_refusal(&project_id) {
                anyhow::bail!(refusal);
            }
            if let Some(backup) = self.inner.options.backup_project.clone() {
                backup(dir.clone()).await?;
            }
            let applied = apply(dir.clone(), timeline_id.clone()).await?;
            self.lock().toast(
                None,
                &format!(
                    "\"{}\" is now \"{}\". The earlier version is kept as \"{}\".",
                    applied.was, applied.main.name, applied.previous.name
                ),
                Tone::Info,
            );
            if let Some(history) = &history
                && let Err(error) = history.record(None).await
            {
                tracing::error!("Couldn't save the edit history: {error:#}");
            }
            self.publish_history(&project_id, None).await;
            let Some(learner) = &learner else {
                return Ok(None);
            };
            let mut locked = self.lock();
            let Some(saved) = locked.saved(&project_id) else {
                return Ok(None);
            };
            // Rounds of feedback: what the user sent since the last approval, less the brief that started the edit.
            let rounds = locked
                .guard
                .hub
                .threads
                .get(learner)
                .map(|entry| {
                    let items = entry.thread.items();
                    let last = items.iter().rposition(|item| {
                        matches!(item, ChatItem::Notice { text, .. } if text.starts_with(APPROVED))
                    });
                    let since = &items[last.map_or(0, |index| index + 1)..];
                    since
                        .iter()
                        .filter(|item| matches!(item, ChatItem::User { text, .. } if !text.starts_with(LEARN)))
                        .count()
                })
                .unwrap_or(0);
            let name = locked.project_name(&saved);
            locked.with_collections(|collections, host| {
                collections.record_edit(host, &saved, &name, rounds.saturating_sub(1) as u32);
            });
            locked.with_thread(learner, |thread, out| {
                thread.note(
                    &format!(
                        "{APPROVED} \"{}\" is now the main timeline \"{}\".",
                        applied.was, applied.main.name
                    ),
                    out,
                );
            });
            Ok(Some(format!(
                "{LEARN} I approved \"{}\" and made it the main timeline \"{}\". Compare your first version of it with the approved one, and propose lessons for next time if there are any.",
                applied.was, applied.main.name
            )))
        }
        .await;
        let learn = match result {
            Ok(learn) => learn,
            Err(error) => {
                self.lock().toast(
                    Some(client),
                    &format!("Couldn't make it the main timeline: {}", error_text(&error)),
                    Tone::Error,
                );
                None
            }
        };
        {
            let mut locked = self.lock();
            locked.guard.hub.applying = None;
            locked.send_meta();
        }
        drop(self.refresh_resolve());
        if let (Some(learn), Some(learner)) = (learn, learner) {
            let mut locked = self.lock();
            let idle = locked
                .guard
                .hub
                .threads
                .get(&learner)
                .is_some_and(|entry| !entry.thread.busy());
            if idle {
                locked.start_turn(&learner, Some(learn), false);
            }
        }
    }

    async fn open_in_resolve(
        &self,
        client: ClientId,
        project_id: String,
        link: ResolveLink,
        local: LocalLink,
    ) {
        let Some(projects) = self.inner.options.resolve_projects.clone() else {
            return;
        };
        let opened = projects.open(&link, &local).await;
        {
            let mut locked = self.lock();
            match opened {
                Ok(opened) => {
                    let open = OpenProject {
                        name: opened.project.name.clone(),
                        id: opened.project.id.clone(),
                        database: opened.database.clone(),
                    };
                    let differs = matches!((&link.project_id, &open.id), (Some(linked), Some(id)) if !linked.is_empty() && !id.is_empty() && linked != id);
                    if differs {
                        locked.toast(
                            Some(client),
                            &format!(
                                "Resolve opened a different project named \"{}\" than the one this project is linked to.",
                                open.name
                            ),
                            Tone::Error,
                        );
                    } else if locked.guard.hub.projects.contains_key(&project_id) {
                        let mut next = refreshed_link(&link, &local, &open);
                        if let Some(folder) = opened.folder {
                            next.local.folder = Some(folder);
                        }
                        locked.set_link(
                            &project_id,
                            next,
                            Some(&format!("Link to Resolve project \"{}\"", open.name)),
                        );
                    }
                }
                Err(error) => locked.toast(
                    Some(client),
                    &format!(
                        "Couldn't open \"{}\" in Resolve: {}",
                        link.name,
                        error_text(&error)
                    ),
                    Tone::Error,
                ),
            }
            locked.guard.hub.opening = None;
        }
        self.refresh_resolve().await;
        self.lock().send_meta();
    }

    async fn load_library(&self, client: ClientId) {
        let Some(projects) = self.inner.options.resolve_projects.clone() else {
            return;
        };
        let library = projects.library().await.map_err(|error| error_text(&error));
        self.lock().send(
            client,
            &ServerMessage::Library {
                library: OrError::from(library),
            },
        );
    }

    async fn send_folders(&self, client: ClientId, request_id: u32, dir: String) {
        let home = std::env::home_dir().unwrap_or_default();
        let listing = browse_folders(&dir, &home)
            .await
            .map_err(|error| error_text(&error));
        self.lock().send(
            client,
            &ServerMessage::Folders {
                request_id,
                listing: OrError::from(listing),
            },
        );
    }

    fn running_turn(&self) -> Result<RunningTurn, String> {
        self.lock().running_turn()
    }

    /// Asks the user during this thread's turn; false when no turn is running.
    async fn request_approval(&self, thread_id: &str, tool: &str, input: &Value) -> bool {
        let sink = self.thread_sink(thread_id);
        match sink {
            Some(sink) => {
                crate::providers::types::TurnSink::request_approval(sink.as_ref(), tool, input)
                    .await
            }
            None => false,
        }
    }

    fn thread_sink(&self, thread_id: &str) -> Option<Arc<crate::thread::ThreadSink>> {
        self.lock()
            .guard
            .hub
            .threads
            .get(thread_id)
            .and_then(|entry| entry.thread.active_sink())
    }

    /// Before an agent changes Resolve: in "ask" mode the change waits for the user, for every agent alike, and the
    /// project is backed up first. A refusal's text, or None when it may go on.
    async fn allow_change(&self, thread_id: &str, tool: &str, input: &Value) -> Option<String> {
        if self.approval_mode() == ApprovalMode::Ask
            && !self
                .request_approval(thread_id, &format!("mcp__{LINKER_MCP_NAME}__{tool}"), input)
                .await
        {
            return Some("The user declined this action.".into());
        }
        let Some(sink) = self.thread_sink(thread_id) else {
            return Some("No turn is running.".into());
        };
        crate::providers::types::TurnSink::backup(sink.as_ref())
            .await
            .err()
    }

    /// Keeps Resolve taken while an agent's call runs: a stopped turn ends before its last call has wound down.
    async fn holding_resolve<T>(&self, run: impl Future<Output = T>) -> T {
        struct Holding<'a>(&'a App);
        impl Drop for Holding<'_> {
            fn drop(&mut self) {
                let mut locked = self.0.lock();
                locked.guard.hub.tool_calls -= 1;
                if locked.guard.hub.tool_calls == 0 {
                    locked.send_meta();
                }
            }
        }
        self.lock().guard.hub.tool_calls += 1;
        let _holding = Holding(self);
        run.await
    }

    /// One of Linker-Edit's Resolve tools, called by an agent's "linker" MCP server. Only the thread with a
    /// turn running may use Resolve, and in "ask" mode changes wait for the user, for every agent alike.
    pub async fn resolve_tool(&self, tool: &str, input: Value) -> ToolReply {
        let definition = find_resolve_tool(tool);
        let (Some(definition), Some(run)) = (definition, self.inner.options.resolve_tools.clone())
        else {
            return ToolReply::error(format!("There is no Resolve tool called {tool}."));
        };
        let turn = match self.running_turn() {
            Ok(turn) => turn,
            Err(refusal) => return ToolReply::error(refusal),
        };
        let setup = self
            .lock()
            .saved(&turn.project_id)
            .is_some_and(|saved| saved.setup);
        if setup && !uses_no_resolve(tool, &input) {
            return ToolReply::error(SETUP_REFUSAL);
        }
        if definition.kind == ResolveToolKind::Write
            && let Some(refusal) = self.allow_change(&turn.thread_id, tool, &input).await
        {
            return ToolReply::error(refusal);
        }
        match self
            .holding_resolve(run(tool.to_string(), input, turn.dir, turn.signal))
            .await
        {
            Ok(output) => ToolReply {
                text: output.text,
                images: output.images,
                is_error: false,
            },
            Err(error) => ToolReply::error(error_text(&error)),
        }
    }

    /// One of the library's tools, called by an agent's "linker" MCP server.
    pub async fn library_tool(&self, tool: &str, input: Map<String, Value>) -> ToolReply {
        let Some(library) = self.inner.options.library.clone() else {
            return ToolReply::error("The library is off in this Linker-Edit.");
        };
        let turn = self.running_turn();
        // A search only reads, so it works between turns too, in the universal library.
        let chain = self
            .lock()
            .library_chain(turn.as_ref().ok().map(|turn| turn.project_id.as_str()));
        if tool == LIBRARY_SEARCH {
            let limit = match input.get("limit").and_then(Value::as_f64) {
                Some(limit) if limit > 0.0 => (limit.floor() as usize).min(100),
                _ => 30,
            };
            let found = library.search(&chain, &query_from_input(&input));
            let mut folders: IndexMap<String, LibraryFolder> = IndexMap::new();
            for folder in chain.iter().flat_map(|link| link.folders.iter()) {
                folders.insert(folder.id.clone(), folder.clone());
            }
            let scanning: Vec<String> = folders
                .values()
                .filter(|folder| library.status(folder).scanning.is_some())
                .map(|folder| format!("\"{}\"", folder.label))
                .collect();
            let mut notes = Vec::new();
            if folders.is_empty() {
                notes.push("The library has no folders yet. The user adds them on Linker-Edit's library page.".to_string());
            }
            if !scanning.is_empty() {
                notes.push(format!(
                    "Still reading {}: more files may show up in a minute.",
                    scanning.join(", ")
                ));
            }
            if found.len() > limit {
                notes.push(format!(
                    "{} more match. Narrow it down, or raise limit.",
                    found.len() - limit
                ));
            }
            let results: Vec<Value> = found
                .iter()
                .take(limit)
                .map(|file| {
                    let folder = &folders[&file.folder_id];
                    serde_json::to_value(describe_file(file, &folder.label, &folder.note))
                        .unwrap_or_default()
                })
                .collect();
            let mut answer = json!({ "results": results, "total": found.len() });
            if !notes.is_empty() {
                answer["note"] = Value::from(notes.join(" "));
            }
            return ToolReply::ok(js::stringify(&answer));
        }
        if tool != LIBRARY_IMPORT {
            return ToolReply::error(format!("There is no library tool called {tool}."));
        }
        let turn = match turn {
            Ok(turn) => turn,
            Err(refusal) => return ToolReply::error(refusal),
        };
        let Some(run) = self.inner.options.resolve_tools.clone() else {
            return ToolReply::error("Linker-Edit can't reach Resolve's tools here.");
        };
        let (project, resolve) = {
            let locked = self.lock();
            let Some(saved) = locked.saved(&turn.project_id) else {
                return ToolReply::error(
                    "No Linker-Edit turn is running, so Resolve calls are refused.",
                );
            };
            (
                (saved.id.clone(), locked.project_name(&saved), saved.setup),
                locked.guard.hub.resolve.clone(),
            )
        };
        if project.2 {
            return ToolReply::error(SETUP_REFUSAL);
        }
        let context = Import {
            app: self.clone(),
            library,
            chain,
            run,
            turn,
            project: (project.0, project.1),
            timeline: match resolve {
                ResolveStatus::Connected { timeline, .. } => timeline,
                _ => None,
            },
        };
        match self.holding_resolve(library_import(&input, &context)).await {
            Ok(result) => ToolReply::ok(js::stringify(&result)),
            Err(error) => ToolReply::error(error_text(&error)),
        }
    }

    /// The agents' study tools: they only read media files, so they need no turn and no Resolve.
    pub async fn study_tool(&self, tool: &str, input: Map<String, Value>) -> ToolReply {
        let Some(study) = self
            .inner
            .options
            .study
            .clone()
            .filter(|_| is_study_tool(tool))
        else {
            return ToolReply::error(format!("There is no study tool called {tool}."));
        };
        let signal = self.running_turn().ok().and_then(|turn| turn.signal);
        match study.run(tool, &input, signal.as_ref()).await {
            Ok(output) => ToolReply {
                text: output.text,
                images: Some(output.images).filter(|images| !images.is_empty()),
                is_error: false,
            },
            Err(error) => ToolReply::error(error_text(&error)),
        }
    }

    /// The agents' brief tools. New rules and lessons steer every later edit, so they wait for the user in full
    /// access too; the agent's own notes on an example don't.
    pub async fn brief_tool(&self, tool: &str, input: Map<String, Value>) -> ToolReply {
        if !is_brief_tool(tool) {
            return ToolReply::error(format!("There is no brief tool called {tool}."));
        }
        let Ok(turn) = self.running_turn() else {
            return ToolReply::error(
                "No Linker-Edit turn is running, so the brief can't be changed.",
            );
        };
        let proposal = {
            let mut locked = self.lock();
            let Some(project) = locked.saved(&turn.project_id) else {
                return ToolReply::error(
                    "No Linker-Edit turn is running, so the brief can't be changed.",
                );
            };
            if tool == SAVE_EXAMPLE {
                let saved = locked.with_collections(|collections, host| {
                    collections.save_example(host, &project, &input)
                });
                return match saved {
                    Ok(text) => ToolReply::ok(text),
                    Err(error) => ToolReply::error(error_text(&error)),
                };
            }
            let name = locked.project_name(&project);
            let collections = &locked.guard.collections;
            let proposal = if tool == SAVE_BRIEF {
                collections.propose_brief(&project, &input)
            } else {
                collections.propose_lesson(&project, &input, &name)
            };
            match proposal {
                Ok(proposal) => proposal,
                Err(error) => return ToolReply::error(error),
            }
        };
        let shows = Value::Object(proposal.shows.clone());
        if !self
            .request_approval(
                &turn.thread_id,
                &format!("mcp__{LINKER_MCP_NAME}__{tool}"),
                &shows,
            )
            .await
        {
            return ToolReply::error(
                "The user declined this change. Ask what they'd like instead; don't propose the same again.",
            );
        }
        let applied = self
            .lock()
            .with_collections(|collections, host| collections.apply_proposal(host, proposal));
        match applied {
            Ok(text) => ToolReply::ok(text),
            Err(error) => ToolReply::error(error_text(&error)),
        }
    }

    /// A library file for the library page to play, or None when it isn't one; see library/serve.rs.
    pub fn library_file(&self, folder_id: &str, rel: &str) -> Option<ServedFile> {
        let folders = self.lock().all_library_folders();
        library_file(&folders, folder_id, rel)
    }

    /// Rescans every library folder, so files added or changed while the app was closed show up.
    pub fn scan_library(&self) -> impl Future<Output = ()> + Send + 'static {
        let folders = self.lock().all_library_folders();
        let library = self.inner.options.library.clone();
        async move {
            if let Some(library) = library {
                library.scan_all(&folders).await;
            }
        }
    }

    /// For the agents' MCP server: the transcripts as text, or what went wrong per file.
    pub async fn transcribe(
        &self,
        files: &[String],
        language: Option<&str>,
    ) -> anyhow::Result<String> {
        let Some(transcriber) = self.inner.options.transcriber.clone() else {
            anyhow::bail!("Transcription is off.");
        };
        let results = transcriber.transcribe(files, language).await;
        Ok(describe_results(&results, DESCRIBE_BUDGET))
    }
}

/// What library_import needs from the turn it runs in.
struct Import {
    app: App,
    library: Arc<Library>,
    chain: Vec<LibraryLink>,
    run: ResolveTools,
    turn: RunningTurn,
    project: (String, String),
    timeline: Option<String>,
}

#[async_trait::async_trait]
impl ImportContext for Import {
    fn find(&self, file: &str) -> Option<LibraryFile> {
        self.library.find(&self.chain, file)
    }

    async fn resolve(&self, tool: &str, input: Value) -> anyhow::Result<String> {
        let output = (self.run)(
            tool.to_string(),
            input,
            self.turn.dir.clone(),
            self.turn.signal.clone(),
        )
        .await?;
        Ok(output.text)
    }

    async fn allow_change(&self, input: Value) -> anyhow::Result<Option<String>> {
        Ok(self
            .app
            .allow_change(&self.turn.thread_id, LIBRARY_IMPORT, &input)
            .await)
    }

    fn record_use(&self, file: &str, project_id: &str, project: &str, timeline: Option<&str>) {
        self.library.record_use(
            file,
            LibraryUse {
                project_id: project_id.to_string(),
                project: project.to_string(),
                timeline: timeline.map(str::to_string),
                at: js::now_iso(),
            },
        );
    }

    fn project(&self) -> (String, String) {
        self.project.clone()
    }

    fn current_timeline(&self) -> Option<String> {
        self.timeline.clone()
    }
}

fn describe_library(link: &ResolveLink, open: &OpenProject) -> String {
    if link.kind == ResolveLinkKind::Cloud {
        return "Blackmagic Cloud".into();
    }
    let name = open
        .database
        .as_ref()
        .map(|database| database.name.as_str())
        .filter(|name| !name.is_empty())
        .map(|name| format!(" \"{name}\""))
        .unwrap_or_default();
    if link.kind == ResolveLinkKind::Network {
        format!("the network database{name}")
    } else {
        format!("the local database{name}")
    }
}

fn same_link(a: &Linked, project: &SavedProject) -> bool {
    let Some(b) = &project.link else {
        return false;
    };
    if a.link.kind != b.kind || a.link.name != b.name {
        return false;
    }
    let known = |id: &Option<String>| {
        id.as_deref()
            .filter(|id| !id.is_empty())
            .map(str::to_string)
    };
    if let (Some(a), Some(b)) = (known(&a.link.project_id), known(&b.project_id)) {
        return a == b;
    }
    match (&a.local.database, &project.local.database) {
        (Some(a), Some(b)) => {
            a.name == b.name
                && a.host.as_deref().unwrap_or_default() == b.host.as_deref().unwrap_or_default()
        }
        _ => true,
    }
}

fn link_from_source(source: &ProjectSource) -> Option<Linked> {
    match source {
        ProjectSource::Cloud { name } => Some(Linked {
            link: ResolveLink {
                kind: ResolveLinkKind::Cloud,
                name: js::trim(name).to_string(),
                project_id: None,
            },
            local: LocalLink::default(),
        }),
        ProjectSource::Library {
            name,
            folder,
            database,
        } => Some(Linked {
            link: ResolveLink {
                kind: link_kind(database.as_ref()),
                name: name.clone(),
                project_id: None,
            },
            local: LocalLink {
                database: database.clone(),
                folder: Some(folder.clone()),
                library: None,
            },
        }),
        _ => None,
    }
}

impl Locked<'_> {
    fn options(&self) -> &AppOptions {
        &self.app.inner.options
    }

    fn saved(&self, project_id: &str) -> Option<SavedProject> {
        self.guard
            .hub
            .projects
            .get(project_id)
            .map(|project| project.saved.clone())
    }

    fn project_name(&self, saved: &SavedProject) -> String {
        // Follows a rename of what it sets up.
        if saved.setup {
            let collection = saved.collection.as_ref();
            let name = collection
                .and_then(|collection| self.guard.collections.get(&collection.id))
                .map(|collection| collection.name.clone())
                .or_else(|| collection.map(|collection| collection.name.clone()))
                .unwrap_or_else(|| "Brand".into());
            return format!("{name} setup");
        }
        saved
            .name
            .clone()
            .or_else(|| saved.link.as_ref().map(|link| link.name.clone()))
            .unwrap_or_else(|| {
                saved
                    .dir
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
    }

    fn resolve_busy(&self) -> bool {
        let hub = &self.guard.hub;
        hub.turn_owner.is_some()
            || hub.preview_job.is_some()
            || hub.opening.is_some()
            || hub.applying.is_some()
            || hub.tool_calls > 0
    }

    /// apply_timeline's start, as the message arrives: Resolve is taken for it at once, or the user is told why not.
    fn begin_apply(
        &mut self,
        client: ClientId,
        project_id: &str,
        timeline_id: &str,
    ) -> Option<Applying> {
        let apply = self.options().apply_timeline.clone()?;
        let project = self.guard.hub.projects.get(project_id)?;
        if timeline_id.is_empty() {
            return None;
        }
        let (dir, history) = (project.saved.dir.clone(), project.history.clone());
        let collection = project.saved.collection.clone();
        if self.resolve_busy() {
            self.toast(
                Some(client),
                "Resolve is busy with a turn, a render or an open. Try again when it's done.",
                Tone::Error,
            );
            return None;
        }
        self.guard.hub.applying = Some(project_id.to_string());
        self.send_meta();
        // The thread the user approved it from learns from the edit, when the project has a brief to learn into.
        let hub = &self.guard.hub;
        let viewed = hub
            .clients
            .get(&client)
            .and_then(|view| view.thread_id.clone())
            .filter(|thread_id| {
                hub.threads
                    .get(thread_id)
                    .is_some_and(|entry| entry.project_id == project_id)
            });
        let has_brief = collection
            .is_some_and(|collection| self.guard.collections.get(&collection.id).is_some());
        Some(Applying {
            apply,
            dir,
            history,
            learner: viewed.filter(|_| has_brief),
        })
    }

    /// open_in_resolve's start: Resolve is taken for the open at once, or the user is told why not.
    fn begin_open(
        &mut self,
        client: ClientId,
        project_id: &str,
    ) -> Option<(ResolveLink, LocalLink)> {
        self.options().resolve_projects.as_ref()?;
        let project = self.guard.hub.projects.get(project_id)?;
        let link = project.saved.link.clone()?;
        let local = project.saved.local.clone();
        if self.resolve_busy() {
            self.toast(
                Some(client),
                "Resolve is busy. Wait until the running turn or render finishes.",
                Tone::Error,
            );
            return None;
        }
        self.guard.hub.opening = Some(project_id.to_string());
        self.send_meta();
        Some((link, local))
    }

    /// Writes the project's rules where agents read them; again before each turn, so they are never stale.
    fn prepare_folder(&self, saved: &SavedProject) {
        let Some(prepare) = &self.options().prepare_folder else {
            return;
        };
        let rules = self.guard.collections.rules_for(saved);
        if let Err(error) = prepare(&saved.dir, &rules) {
            tracing::error!("{error:#}");
        }
    }

    fn add_project(&mut self, saved: SavedProject, threads: Vec<SavedThread>) -> String {
        self.prepare_folder(&saved);
        let id = saved.id.clone();
        // A setup project has no Resolve project, so nothing to keep a history of.
        let history = match (&self.options().create_history, saved.setup) {
            (Some(create), false) => {
                let me = self.app.inner.me.clone();
                let project_id = id.clone();
                let check: ProjectCheck = Arc::new(move |open: &OpenSnapshotProject| {
                    let inner = me.upgrade()?;
                    let app = App { inner };
                    let locked = app.lock();
                    let link = locked
                        .guard
                        .hub
                        .projects
                        .get(&project_id)?
                        .saved
                        .link
                        .clone()?;
                    let same = match (&link.project_id, &open.id) {
                        (Some(linked), Some(open_id))
                            if !linked.is_empty() && !open_id.is_empty() =>
                        {
                            linked == open_id
                        }
                        _ => link.name == open.name,
                    };
                    (!same).then(|| {
                        format!(
                            "Resolve has \"{}\" open, not this project's \"{}\", so its timelines weren't saved here.",
                            open.name, link.name
                        )
                    })
                });
                Some(create(&saved.dir, check))
            }
            _ => None,
        };
        self.guard
            .hub
            .projects
            .insert(id.clone(), ProjectState { saved, history });
        for thread in threads {
            self.add_thread(&id, thread);
        }
        id
    }

    fn add_thread(&mut self, project_id: &str, saved: SavedThread) -> String {
        let project = &self.guard.hub.projects[project_id];
        let (project_saved, history) = (project.saved.clone(), project.history.clone());
        let id = saved.id.clone();
        let context = Arc::new(TurnContext {
            app: self.app.inner.me.clone(),
            thread_id: id.clone(),
            project_id: project_id.to_string(),
            dir: project_saved.dir.clone(),
            history,
        });
        let session = self
            .app
            .session_for(&project_saved, saved.provider, saved.resume_id.clone());
        let thread = Thread::new(saved, session);
        self.guard.hub.threads.insert(
            id.clone(),
            ThreadEntry {
                thread,
                project_id: project_id.to_string(),
                context,
            },
        );
        id
    }

    fn new_thread(&mut self, project_id: &str) -> String {
        // An untouched new thread is reused rather than piling up.
        let empty = self.project_threads(project_id).into_iter().find(|id| {
            let thread = &self.guard.hub.threads[id].thread;
            thread.empty() && !thread.archived() && !thread.busy()
        });
        if let Some(empty) = empty {
            return empty;
        }
        let now = js::now_iso();
        let provider = self.guard.hub.last_provider;
        self.add_thread(
            project_id,
            SavedThread {
                id: uuid::Uuid::new_v4().to_string(),
                title: DEFAULT_TITLE.into(),
                provider,
                resume_id: None,
                items: Vec::new(),
                context: None,
                created_at: now.clone(),
                updated_at: now,
                archived_at: None,
                sidebar: NEW_THREAD_SIDEBAR,
            },
        )
    }

    /// A project's threads, newest first.
    fn project_threads(&self, project_id: &str) -> Vec<String> {
        let mut threads: Vec<&ThreadEntry> = self
            .guard
            .hub
            .threads
            .values()
            .filter(|entry| entry.project_id == project_id)
            .collect();
        threads.sort_by(|a, b| b.thread.updated_at().cmp(a.thread.updated_at()));
        threads
            .into_iter()
            .map(|entry| entry.thread.id.clone())
            .collect()
    }

    /// Runs f on a live thread and carries out what it changed.
    fn with_thread<R>(
        &mut self,
        thread_id: &str,
        f: impl FnOnce(&mut Thread, &mut Events) -> R,
    ) -> Option<R> {
        let entry = self.guard.hub.threads.get_mut(thread_id)?;
        if entry.thread.disposed() {
            return None;
        }
        let mut events = Vec::new();
        let result = f(&mut entry.thread, &mut events);
        self.carry_out(thread_id, events);
        Some(result)
    }

    fn carry_out(&mut self, thread_id: &str, events: Events) {
        let mut changed = false;
        for event in events {
            match event {
                ThreadEvent::Changed => changed = true,
                ThreadEvent::Item(item) => self.send_item(thread_id, item),
                ThreadEvent::Refused { text, reason } => {
                    let viewers: Vec<ClientId> = self.viewers(thread_id);
                    for client in viewers {
                        self.send(
                            client,
                            &ServerMessage::Refused {
                                thread_id: thread_id.to_string(),
                                text: text.clone(),
                            },
                        );
                        self.send(
                            client,
                            &ServerMessage::Toast {
                                text: reason.clone(),
                                tone: Tone::Error,
                            },
                        );
                    }
                }
                ThreadEvent::Persist(record) => {
                    let hub = &self.guard.hub;
                    if let Some(project) = hub
                        .threads
                        .get(thread_id)
                        .and_then(|entry| hub.projects.get(&entry.project_id))
                    {
                        self.options().store.save_thread(&project.saved, &record);
                    }
                }
            }
        }
        if changed {
            self.send_meta();
        }
    }

    fn viewers(&self, thread_id: &str) -> Vec<ClientId> {
        self.guard
            .hub
            .clients
            .iter()
            .filter(|(_, view)| view.thread_id.as_deref() == Some(thread_id))
            .map(|(id, _)| *id)
            .collect()
    }

    /// The project whose Resolve project is open, if Linker-Edit has one.
    fn owner_of(&self, open: &OpenProject) -> Option<String> {
        self.guard
            .hub
            .projects
            .values()
            .find(|project| {
                project
                    .saved
                    .link
                    .as_ref()
                    .is_some_and(|link| is_linked_project(link, &project.saved.local, open))
            })
            .map(|project| project.saved.id.clone())
    }

    /// Why a thread of this project can't run against Resolve now; None when it can.
    fn project_refusal(&self, project_id: &str) -> Option<String> {
        let open = open_project(&self.guard.hub.resolve)?;
        let saved = &self.guard.hub.projects.get(project_id)?.saved;
        if saved.setup {
            return None;
        }
        if let Some(link) = &saved.link {
            if is_linked_project(link, &saved.local, &open) {
                return None;
            }
            return Some(format!(
                "Resolve has \"{}\" open, not this project's \"{}\". Open \"{}\" in Resolve to continue.",
                open.name, link.name, link.name
            ));
        }
        let owner = self.owner_of(&open)?;
        let owner = &self.guard.hub.projects[&owner].saved;
        Some(format!(
            "Resolve has \"{}\" open, which belongs to the project \"{}\". Switch to that project, or open another one in Resolve.",
            open.name,
            self.project_name(owner)
        ))
    }

    fn set_link(&mut self, project_id: &str, next: Linked, commit_message: Option<&str>) {
        let Some(project) = self.guard.hub.projects.get_mut(project_id) else {
            return;
        };
        let link_changed = project.saved.link.as_ref() != Some(&next.link);
        project.saved.link = Some(next.link);
        project.saved.local = keep_library(&project.saved.local, &next.local);
        self.app.inner.options.store.save_project(&project.saved);
        if link_changed && let Some(message) = commit_message {
            commit_project_file(project.history.clone(), message.to_string());
        }
        self.send_meta();
    }

    fn summaries(&self) -> Vec<ProjectSummary> {
        let open = open_project(&self.guard.hub.resolve);
        self.guard
            .hub
            .projects
            .values()
            .map(|project| {
                let saved = &project.saved;
                ProjectSummary {
                    id: saved.id.clone(),
                    name: self.project_name(saved),
                    dir: saved.dir.to_string_lossy().into_owned(),
                    link: saved.link.clone(),
                    local: saved.local.clone(),
                    open: match (&open, &saved.link) {
                        (Some(open), Some(link)) => is_linked_project(link, &saved.local, open),
                        _ => false,
                    },
                    threads: self
                        .project_threads(&saved.id)
                        .iter()
                        .map(|id| self.guard.hub.threads[id].thread.summary())
                        .collect(),
                    setup: saved.setup,
                    sidebar: ProjectSidebarState {
                        icon: saved.icon.clone(),
                        pinned: saved.pinned,
                        archived: saved.archived,
                    },
                }
            })
            .collect()
    }

    fn blocked_for(&self, thread_id: &str, project_id: &str) -> Option<Blocked> {
        let hub = &self.guard.hub;
        if hub.threads[thread_id].thread.busy() {
            return None;
        }
        let blocked = |message: String| {
            Some(Blocked {
                message,
                can_open: false,
            })
        };
        if let Some(opening) = &hub.opening {
            let name = hub.projects.get(opening).map_or_else(
                || "the project".to_string(),
                |target| self.project_name(&target.saved),
            );
            return blocked(format!("Opening \"{name}\" in Resolve…"));
        }
        if hub.applying.is_some() {
            return blocked("Making the AI timeline the main one…".into());
        }
        if hub.preview_job.is_some() {
            return blocked("Resolve is rendering a preview. Send when it's done, or stop it in the preview panel.".into());
        }
        if let Some(owner) = hub.turn_owner.as_ref().and_then(|id| hub.threads.get(id)) {
            let place = if owner.project_id == project_id {
                String::new()
            } else {
                format!(
                    " ({})",
                    self.project_name(&hub.projects[&owner.project_id].saved)
                )
            };
            return blocked(format!(
                "{} is working in \"{}\"{place}. One thread works in Resolve at a time.",
                provider_name(owner.thread.provider()),
                owner.thread.title()
            ));
        }
        if hub.tool_calls > 0 {
            return blocked("Waiting for Resolve to finish the stopped turn's last step…".into());
        }
        let refusal = self.project_refusal(project_id)?;
        let linked = hub.projects[project_id].saved.link.is_some();
        Some(Blocked {
            message: refusal,
            can_open: linked && self.options().resolve_projects.is_some(),
        })
    }

    fn meta_for(&self, thread_id: Option<&str>) -> AppMeta {
        let hub = &self.guard.hub;
        let entry = thread_id.and_then(|id| hub.threads.get(id).map(|entry| (id, entry)));
        let project = entry.and_then(|(_, entry)| hub.projects.get(&entry.project_id));
        let options = self.options();
        let project_id = entry.map(|(_, entry)| entry.project_id.clone());
        AppMeta {
            projects: self.summaries(),
            thread_id: entry.map(|(id, _)| id.to_string()),
            project_id: project_id.clone(),
            provider: entry.map_or(hub.last_provider, |(_, entry)| entry.thread.provider()),
            default_provider: hub.last_provider,
            approval_mode: self.app.approval_mode(),
            busy: entry.is_some_and(|(_, entry)| entry.thread.busy()),
            approvals: entry
                .map(|(_, entry)| entry.thread.approvals())
                .unwrap_or_default(),
            resolve: hub.resolve.clone(),
            resolve_here: hub.resolve_here,
            version: options.version.clone(),
            update: hub.update.clone(),
            app_update: options
                .app_updates
                .as_ref()
                .and_then(|updates| updates.status()),
            workspace_dir: project.map(|project| project.saved.dir.to_string_lossy().into_owned()),
            preview: if hub.preview_project == project_id {
                hub.preview.clone()
            } else {
                PreviewState::Idle
            },
            context: entry.and_then(|(_, entry)| entry.thread.context().cloned()),
            blocked: entry.and_then(|(id, entry)| self.blocked_for(id, &entry.project_id)),
            opening: hub.opening.clone(),
            applying: hub.applying.clone(),
            transcription: options
                .transcriber
                .as_ref()
                .map_or(TranscriptionStatus::Idle { ready: None }, |transcriber| {
                    transcriber.status()
                }),
            remote: options.remote.as_ref().map_or_else(
                || RemoteAccess::Unavailable {
                    hint: "Remote access isn't available in this build.".into(),
                },
                |remote| remote.status(),
            ),
            connect: options
                .connect
                .as_ref()
                .map_or_else(no_connect, |connect| connect.status()),
            computer: gethostname::gethostname().to_string_lossy().into_owned(),
            setup: options.setup.clone(),
            linker: options
                .linker
                .as_ref()
                .map_or(LinkerAccount::SignedOut { problem: None }, |linker| {
                    linker.status()
                }),
            collections: self.guard.collections.summaries(&ReadHost(hub)),
        }
    }

    fn send(&self, client: ClientId, message: &ServerMessage) {
        if let Some(entry) = self.guard.hub.clients.get(&client) {
            entry.client.send(js::stringify(message));
        }
    }

    fn send_all(&self, message: &ServerMessage) {
        let data = js::stringify(message);
        for entry in self.guard.hub.clients.values() {
            entry.client.send(data.clone());
        }
    }

    fn send_meta(&mut self) {
        for (_, entry) in self.guard.hub.clients.iter() {
            let meta = self.meta_for(entry.thread_id.as_deref());
            entry.client.send(js::stringify(&ServerMessage::Meta {
                meta: Box::new(meta),
            }));
        }
    }

    fn send_snapshot(&self, client: ClientId) {
        let Some(view) = self.guard.hub.clients.get(&client) else {
            return;
        };
        let thread_id = view.thread_id.as_deref();
        let items = thread_id
            .and_then(|id| self.guard.hub.threads.get(id))
            .map(|entry| entry.thread.items().to_vec())
            .unwrap_or_default();
        let state = AppState {
            meta: self.meta_for(thread_id),
            items,
        };
        self.send(
            client,
            &ServerMessage::Snapshot {
                state: Box::new(state),
            },
        );
    }

    fn send_item(&self, thread_id: &str, item: ChatItem) {
        let message = ServerMessage::Item {
            thread_id: thread_id.to_string(),
            item,
        };
        for client in self.viewers(thread_id) {
            self.send(client, &message);
        }
    }

    fn toast(&self, client: Option<ClientId>, text: &str, tone: Tone) {
        let message = ServerMessage::Toast {
            text: text.to_string(),
            tone,
        };
        match client {
            Some(client) => self.send(client, &message),
            None => self.send_all(&message),
        }
    }

    /// Where a client lands: the project open in Resolve first, then its latest thread, or a new one.
    fn default_thread(&mut self, prefer_project: Option<&str>) -> Option<String> {
        let hub = &self.guard.hub;
        let owner = open_project(&hub.resolve).and_then(|open| self.owner_of(&open));
        let project_id = prefer_project
            .filter(|id| hub.projects.contains_key(*id))
            .map(str::to_string)
            .or(owner)
            .or_else(|| hub.projects.keys().next().cloned())?;
        let existing = self
            .project_threads(&project_id)
            .into_iter()
            .find(|id| !self.guard.hub.threads[id].thread.archived());
        Some(existing.unwrap_or_else(|| self.new_thread(&project_id)))
    }

    fn select(&mut self, client: ClientId, thread_id: Option<String>) {
        if let Some(view) = self.guard.hub.clients.get_mut(&client) {
            view.thread_id = thread_id;
        }
        self.send_snapshot(client);
        self.send_meta();
    }

    /// Clients looking at a thread that went away move to another thread of the same project.
    fn move_viewers(&mut self, thread_id: &str, project_id: &str) {
        for client in self.viewers(thread_id) {
            let next = self.default_thread(Some(project_id));
            if let Some(view) = self.guard.hub.clients.get_mut(&client) {
                view.thread_id = next;
            }
            self.send_snapshot(client);
        }
        self.send_meta();
    }

    fn remove_thread(&mut self, thread_id: &str) {
        let Some(mut entry) = self.guard.hub.threads.shift_remove(thread_id) else {
            return;
        };
        entry.thread.dispose();
        if let Some(project) = self.guard.hub.projects.get(&entry.project_id) {
            self.options()
                .store
                .delete_thread(&project.saved, thread_id);
        }
        self.move_viewers(thread_id, &entry.project_id);
    }

    /// A project's PROJECT.md as it is on disk. A setup project has none.
    fn publish_project_file(&self, project_id: &str, client: Option<ClientId>) {
        let Some(saved) = self
            .guard
            .hub
            .projects
            .get(project_id)
            .map(|project| &project.saved)
        else {
            return;
        };
        if saved.setup {
            return;
        }
        let message = ServerMessage::ProjectFile {
            project_id: project_id.to_string(),
            file: saved
                .dir
                .join(PROJECT_NOTES_FILE)
                .to_string_lossy()
                .into_owned(),
            text: read_project_file(&saved.dir),
        };
        match client {
            Some(client) => self.send(client, &message),
            None => self.send_all(&message),
        }
    }

    fn save_project_file(&mut self, client: ClientId, project_id: &str, text: &str) {
        let Some(saved) = self.saved(project_id).filter(|saved| !saved.setup) else {
            return;
        };
        if let Some(refusal) = write_project_file(&saved.dir, text) {
            return self.toast(Some(client), &refusal, Tone::Error);
        }
        // Codex reads it from AGENTS.md, so that follows at once.
        self.prepare_folder(&saved);
        self.publish_project_file(project_id, None);
    }

    fn render_preview(
        &mut self,
        client: ClientId,
        project_id: Option<String>,
        timeline_id: Option<String>,
    ) {
        let Some(renderer) = self.options().preview.clone() else {
            return;
        };
        if self.resolve_busy() {
            return self.toast(
                Some(client),
                "Resolve is busy with a turn, a render or an open. Render when it's done.",
                Tone::Error,
            );
        }
        // Resolve renders whatever project it has open: only this thread's own.
        if let Some(refusal) = project_id
            .as_deref()
            .and_then(|id| self.project_refusal(id))
        {
            return self.toast(Some(client), &refusal, Tone::Error);
        }
        self.guard.hub.preview_project = project_id;
        self.guard.hub.preview = PreviewState::Rendering {
            timeline: None,
            percent: 0.0,
        };
        self.send_meta();
        let me = self.app.inner.me.clone();
        let job = renderer.render(
            timeline_id,
            Arc::new(move |event| {
                if let Some(inner) = me.upgrade() {
                    App { inner }.run_locked(move |locked| locked.preview_event(event));
                }
            }),
        );
        self.guard.hub.preview_job = Some(job);
    }

    fn preview_event(&mut self, event: PreviewEvent) {
        let hub = &mut self.guard.hub;
        match event {
            PreviewEvent::Start { timeline } => {
                hub.preview = PreviewState::Rendering {
                    timeline: Some(PreviewTimeline {
                        id: timeline.id,
                        name: timeline.name,
                    }),
                    percent: 0.0,
                };
            }
            PreviewEvent::Progress { percent } => {
                if let PreviewState::Rendering { percent: now, .. } = &mut hub.preview {
                    *now = percent;
                }
            }
            PreviewEvent::Done { .. } | PreviewEvent::Error { .. } => {
                let timeline = match &hub.preview {
                    PreviewState::Rendering { timeline, .. } => timeline.clone(),
                    _ => None,
                };
                hub.preview_job = None;
                hub.preview = match (event, timeline) {
                    (
                        PreviewEvent::Error {
                            stopped: Some(true),
                            ..
                        },
                        _,
                    ) => PreviewState::Idle,
                    (PreviewEvent::Error { message, .. }, _) => PreviewState::Error { message },
                    (_, None) => PreviewState::Error {
                        message: "Resolve rendered without naming the timeline.".into(),
                    },
                    (PreviewEvent::Done { file }, Some(timeline)) => {
                        let name = file
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        PreviewState::Ready {
                            timeline,
                            url: format!("/preview/{}", encode_uri_component(&name)),
                            rendered_at: js::now_iso(),
                            outdated: false,
                        }
                    }
                    (_, Some(_)) => unreachable!("only done and error end a render"),
                };
            }
        }
        self.send_meta();
    }

    fn prepare_transcription(&self) {
        let Some(transcriber) = &self.options().transcriber else {
            return;
        };
        let ready = matches!(
            transcriber.status(),
            TranscriptionStatus::Idle { ready: Some(false) }
        );
        if !self.guard.hub.resolve_here || !ready {
            return;
        }
        let setup = transcriber.setup();
        tokio::spawn(async move {
            if let Err(error) = setup.await {
                tracing::error!("Couldn't set up transcription: {error}");
            }
        });
    }

    fn set_resolve_here(&mut self, value: bool) {
        if value == self.guard.hub.resolve_here {
            return;
        }
        self.guard.hub.resolve_here = value;
        if let Some(settings) = &self.options().settings {
            settings.save(Settings {
                resolve_here: Some(value),
                ..Settings::default()
            });
        }
        if value {
            self.prepare_transcription();
        } else if let Some(transcriber) = &self.options().transcriber {
            transcriber.cancel_setup();
        }
        self.send_meta();
    }

    fn apply_resolve_status(&mut self, status: ResolveStatus) {
        let setup_needed = matches!(status, ResolveStatus::SetupNeeded { .. });
        self.guard.hub.resolve = status;
        // Resolve turned up on a computer that was said to have none, so it's an edit computer after all.
        if !self.guard.hub.resolve_here && !setup_needed {
            self.set_resolve_here(true);
        }
        // Learns the project's id once it is seen open, and follows renames made in Resolve.
        let open = open_project(&self.guard.hub.resolve);
        if let Some(open) = open
            && let Some(owner) = self.owner_of(&open)
        {
            let saved = &self.guard.hub.projects[&owner].saved;
            if let Some(link) = &saved.link {
                let next = refreshed_link(link, &saved.local, &open);
                let current = Linked {
                    link: link.clone(),
                    local: saved.local.clone(),
                };
                if next != current {
                    let message = if next.link.name != link.name {
                        format!("Resolve project renamed to \"{}\"", open.name)
                    } else {
                        format!("Link to Resolve project \"{}\"", open.name)
                    };
                    self.set_link(&owner, next, Some(&message));
                }
            }
        }
        self.send_meta();
    }

    fn add_project_from(&mut self, client: ClientId, source: ProjectSource) {
        let project_id = if let ProjectSource::Folder { dir } = &source {
            let home = std::env::home_dir().unwrap_or_default();
            match self
                .options()
                .store
                .adopt(Path::new(&expand_home(dir, &home)))
            {
                Ok(adopted) => self.add_project(adopted.project, adopted.threads),
                Err(error) => return self.toast(Some(client), &error_text(&error), Tone::Error),
            }
        } else {
            let next = if let ProjectSource::Open = source {
                let Some(open) = open_project(&self.guard.hub.resolve) else {
                    return self.toast(
                        Some(client),
                        "Open a project in Resolve first.",
                        Tone::Error,
                    );
                };
                link_from_open(&open)
            } else {
                let name = match &source {
                    ProjectSource::Cloud { name } | ProjectSource::Library { name, .. } => name,
                    _ => unreachable!("open and folder are handled above"),
                };
                if js::trim(name).is_empty() {
                    let text = if matches!(source, ProjectSource::Cloud { .. }) {
                        "A cloud project needs its name."
                    } else {
                        "Pick a project."
                    };
                    return self.toast(Some(client), text, Tone::Error);
                }
                link_from_source(&source).expect("cloud or library")
            };
            let existing = self
                .guard
                .hub
                .projects
                .values()
                .find(|project| same_link(&next, &project.saved))
                .map(|project| project.saved.clone());
            if let Some(existing) = existing {
                let text = format!(
                    "\"{}\" is already in Linker-Edit.",
                    self.project_name(&existing)
                );
                self.toast(Some(client), &text, Tone::Info);
                let thread = self.default_thread(Some(&existing.id));
                return self.select(client, thread);
            }
            let message = format!("Link to Resolve project \"{}\"", next.link.name);
            let created = self.options().store.create(NewProject {
                name: None,
                link: Some(next.link),
                local: next.local,
                collection: None,
                setup: false,
            });
            let id = self.add_project(created, Vec::new());
            commit_project_file(self.guard.hub.projects[&id].history.clone(), message);
            id
        };
        let thread = self.default_thread(Some(&project_id));
        self.select(client, thread);
    }

    fn update_project(&mut self, project_id: &str, name: Option<&str>) {
        let Some(project) = self.guard.hub.projects.get_mut(project_id) else {
            return;
        };
        let before = project.saved.name.clone();
        if let Some(name) = name {
            project.saved.name = Some(js::trim(name).to_string()).filter(|name| !name.is_empty());
        }
        self.app.inner.options.store.save_project(&project.saved);
        if project.saved.name != before {
            let saved = project.saved.clone();
            let history = project.history.clone();
            let message = format!("Rename project to \"{}\"", self.project_name(&saved));
            commit_project_file(history, message);
        }
        self.send_meta();
    }

    fn relink(&mut self, client: ClientId, project_id: &str) {
        let Some(saved) = self.saved(project_id) else {
            return;
        };
        if saved.setup {
            return;
        }
        let Some(open) = open_project(&self.guard.hub.resolve) else {
            return self.toast(
                Some(client),
                "Open the project in Resolve first, then link it.",
                Tone::Error,
            );
        };
        if let Some(owner) = self.owner_of(&open).filter(|owner| owner != project_id) {
            let owner = self.guard.hub.projects[&owner].saved.clone();
            let text = format!(
                "\"{}\" already belongs to the project \"{}\".",
                open.name,
                self.project_name(&owner)
            );
            return self.toast(Some(client), &text, Tone::Error);
        }
        let next = link_from_open(&open);
        let local = LocalLink {
            database: next.local.database.or(saved.local.database.clone()),
            ..saved.local.clone()
        };
        self.set_link(
            project_id,
            Linked {
                link: next.link,
                local,
            },
            Some(&format!("Link to Resolve project \"{}\"", open.name)),
        );
    }

    fn remove_project(&mut self, project_id: &str) {
        if !self.guard.hub.projects.contains_key(project_id) {
            return;
        }
        for thread_id in self.project_threads(project_id) {
            if let Some(mut entry) = self.guard.hub.threads.shift_remove(&thread_id) {
                entry.thread.dispose();
            }
        }
        self.guard.hub.projects.shift_remove(project_id);
        self.options().store.remove_project(project_id);
        let lost: Vec<ClientId> = self
            .guard
            .hub
            .clients
            .iter()
            .filter(|(_, view)| {
                view.thread_id
                    .as_ref()
                    .is_some_and(|id| !self.guard.hub.threads.contains_key(id))
            })
            .map(|(id, _)| *id)
            .collect();
        for client in lost {
            let next = self.default_thread(None);
            if let Some(view) = self.guard.hub.clients.get_mut(&client) {
                view.thread_id = next;
            }
            self.send_snapshot(client);
        }
        self.send_meta();
    }

    fn claim(&mut self, thread_id: &str) -> bool {
        if self.resolve_busy() {
            return false;
        }
        self.guard.hub.turn_owner = Some(thread_id.to_string());
        true
    }

    /// thread.send (text) or thread.compact (None): takes Resolve for the turn now, and runs it in the background.
    fn start_turn(&mut self, thread_id: &str, text: Option<String>, compact_first: bool) {
        let Some(entry) = self.guard.hub.threads.get(thread_id) else {
            return;
        };
        if !entry.thread.can_start(text.as_deref()) {
            return;
        }
        let context = entry.context.clone();
        // Resolve does one thing at a time: a render switches timelines and pages while it runs.
        if !self.claim(thread_id) {
            let reason = "Resolve is busy with another thread's turn or a render. Send again when it's done.";
            self.with_thread(thread_id, |thread, out| {
                thread.refused(text, reason.to_string(), out);
            });
            return;
        }
        let Some(signal) = self.with_thread(thread_id, |thread, out| thread.begin(out)) else {
            return;
        };
        tokio::spawn(run_turn(context, text, compact_first, signal));
    }

    fn running_turn(&self) -> Result<RunningTurn, String> {
        let hub = &self.guard.hub;
        let entry = hub
            .turn_owner
            .as_ref()
            .and_then(|id| hub.threads.get(id).map(|entry| (id, entry)));
        let dir = entry.and_then(|(_, entry)| {
            hub.projects
                .get(&entry.project_id)
                .map(|project| project.saved.dir.clone())
        });
        let (Some((thread_id, entry)), Some(dir)) = (entry, dir) else {
            return Err("No Linker-Edit turn is running, so Resolve calls are refused.".into());
        };
        if !entry.thread.busy() {
            return Err("No Linker-Edit turn is running, so Resolve calls are refused.".into());
        }
        let signal = entry.thread.signal();
        if signal.as_ref().is_some_and(CancellationToken::is_cancelled) {
            return Err("The user stopped this turn.".into());
        }
        Ok(RunningTurn {
            thread_id: thread_id.clone(),
            project_id: entry.project_id.clone(),
            dir,
            signal,
        })
    }

    // --- Library ---

    /// The libraries a project gets, least specific first. Workspaces and series will add theirs in between.
    fn library_chain(&self, project_id: Option<&str>) -> Vec<LibraryLink> {
        let mut chain = vec![LibraryLink {
            scope: LibraryScope::Universal,
            label: "Universal".into(),
            folders: self
                .options()
                .library
                .as_ref()
                .map(|library| library.folders())
                .unwrap_or_default(),
        }];
        if let Some(project) = project_id.and_then(|id| self.guard.hub.projects.get(id)) {
            chain.push(LibraryLink {
                scope: LibraryScope::Project {
                    project_id: project.saved.id.clone(),
                },
                label: self.project_name(&project.saved),
                folders: project_folders(&project.saved.local),
            });
        }
        chain
    }

    fn all_library_folders(&self) -> Vec<LibraryFolder> {
        let mut folders = self
            .options()
            .library
            .as_ref()
            .map(|library| library.folders())
            .unwrap_or_default();
        for project in self.guard.hub.projects.values() {
            folders.extend(project_folders(&project.saved.local));
        }
        folders
    }

    fn library_link(&self, scope: &LibraryScope) -> Option<LibraryLink> {
        match scope {
            LibraryScope::Universal => self.library_chain(None).into_iter().next(),
            LibraryScope::Project { project_id } => self
                .guard
                .hub
                .projects
                .contains_key(project_id)
                .then(|| self.library_chain(Some(project_id)).pop())
                .flatten(),
        }
    }

    fn save_library_folders(&mut self, scope: &LibraryScope, folders: Vec<LibraryFolder>) {
        match scope {
            LibraryScope::Universal => {
                if let Some(library) = &self.options().library {
                    library.save_folders(folders);
                }
            }
            LibraryScope::Project { project_id } => {
                let Some(project) = self.guard.hub.projects.get_mut(project_id) else {
                    return;
                };
                project.saved.local = with_project_folders(&project.saved.local, &folders);
                self.app.inner.options.store.save_project(&project.saved);
            }
        }
    }

    fn library_view(&self, client: ClientId, wanted: &LibraryWanted) -> LibraryView {
        let current = self
            .guard
            .hub
            .clients
            .get(&client)
            .and_then(|view| view.thread_id.as_ref());
        let project_id = current
            .and_then(|id| self.guard.hub.threads.get(id))
            .map(|entry| entry.project_id.clone());
        let link = self
            .library_link(&wanted.scope)
            .unwrap_or_else(|| self.library_chain(None).remove(0));
        let mut scopes: Vec<LibraryScopeOption> = self
            .library_chain(project_id.as_deref())
            .into_iter()
            .map(|link| LibraryScopeOption {
                scope: link.scope,
                label: link.label,
            })
            .collect();
        if matches!(link.scope, LibraryScope::Project { .. })
            && !scopes.iter().any(|entry| entry.scope == link.scope)
        {
            scopes.push(LibraryScopeOption {
                scope: link.scope.clone(),
                label: link.label.clone(),
            });
        }
        let library = self.options().library.as_ref();
        let files = library
            .map(|library| library.search(std::slice::from_ref(&link), &wanted.query))
            .unwrap_or_default();
        LibraryView {
            scope: link.scope.clone(),
            scopes,
            folders: link
                .folders
                .iter()
                .filter_map(|folder| library.map(|library| library.status(folder)))
                .collect(),
            total: files.len() as u32,
            files: files.into_iter().take(LIBRARY_PAGE).collect(),
        }
    }

    fn send_library_views(&mut self) {
        let views: Vec<(ClientId, LibraryWanted)> = self
            .guard
            .hub
            .library_views
            .iter()
            .map(|(client, wanted)| (*client, wanted.clone()))
            .collect();
        for (client, wanted) in views {
            let view = self.library_view(client, &wanted);
            self.send(
                client,
                &ServerMessage::LibraryView(
                    linker_edit_protocol::LibraryServerMessage::LibraryView { view },
                ),
            );
        }
    }

    /// The library page's messages: what it shows, and changes to folders and files. Files on disk are never touched.
    fn library_message(&mut self, client: ClientId, message: LibraryClientMessage) {
        let Some(library) = self.options().library.clone() else {
            return;
        };
        let home = std::env::home_dir().unwrap_or_default();
        let result: anyhow::Result<bool> = (|| {
            match message {
                LibraryClientMessage::LibraryOpen { scope, query } => {
                    self.guard
                        .hub
                        .library_views
                        .insert(client, LibraryWanted { scope, query });
                }
                LibraryClientMessage::LibraryClose => {
                    self.guard.hub.library_views.shift_remove(&client);
                    return Ok(false);
                }
                LibraryClientMessage::LibraryAddFolder { scope, mut folder } => {
                    let Some(link) = self.library_link(&scope) else {
                        return Ok(false);
                    };
                    folder.path = expand_home(&folder.path, &home);
                    let folder = new_folder(&folder, &link.folders)?;
                    let mut folders = link.folders.clone();
                    folders.push(folder.clone());
                    self.save_library_folders(&link.scope, folders);
                    tokio::spawn(library.scan(&folder));
                }
                LibraryClientMessage::LibraryUpdateFolder {
                    scope,
                    folder_id,
                    changes,
                } => {
                    let Some(link) = self.library_link(&scope) else {
                        return Ok(false);
                    };
                    let Some(old) = link
                        .folders
                        .iter()
                        .find(|folder| folder.id == folder_id)
                        .cloned()
                    else {
                        return Ok(false);
                    };
                    let mut value = serde_json::to_value(&old)?;
                    if let Value::Object(fields) = serde_json::to_value(&changes)? {
                        value.as_object_mut().expect("an object").extend(fields);
                    }
                    value["id"] = Value::from(old.id.clone());
                    value["path"] = Value::from(match &changes.path {
                        None => old.path.clone(),
                        Some(path) => expand_home(path, &home),
                    });
                    let next = clean_folder(&value)
                        .ok_or_else(|| anyhow::anyhow!("Pick a kind and a folder."))?;
                    if next.path != old.path {
                        check_folder(&next, &link.folders)?;
                    }
                    let folders = link
                        .folders
                        .iter()
                        .map(|folder| {
                            if folder.id == old.id {
                                next.clone()
                            } else {
                                folder.clone()
                            }
                        })
                        .collect();
                    self.save_library_folders(&link.scope, folders);
                    if next.path != old.path || next.kind != old.kind {
                        library.forget(&old);
                        tokio::spawn(library.scan(&next));
                    }
                }
                LibraryClientMessage::LibraryRemoveFolder { scope, folder_id } => {
                    let Some(link) = self.library_link(&scope) else {
                        return Ok(false);
                    };
                    let Some(old) = link
                        .folders
                        .iter()
                        .find(|folder| folder.id == folder_id)
                        .cloned()
                    else {
                        return Ok(false);
                    };
                    let folders = link
                        .folders
                        .iter()
                        .filter(|folder| folder.id != old.id)
                        .cloned()
                        .collect();
                    self.save_library_folders(&link.scope, folders);
                    library.forget(&old);
                }
                LibraryClientMessage::LibraryRescan { folder_id } => {
                    if let Some(folder) = self
                        .all_library_folders()
                        .into_iter()
                        .find(|candidate| candidate.id == folder_id)
                    {
                        tokio::spawn(library.scan(&folder));
                    }
                    return Ok(false);
                }
                LibraryClientMessage::LibraryUpdateFile {
                    folder_id,
                    rel,
                    star,
                    tags,
                    note,
                } => {
                    let folder = self
                        .all_library_folders()
                        .into_iter()
                        .find(|candidate| candidate.id == folder_id);
                    let parts: Vec<&str> = rel.split('/').collect();
                    let Some(folder) = folder else {
                        return Ok(false);
                    };
                    if parts
                        .iter()
                        .any(|part| part.is_empty() || *part == "." || *part == "..")
                    {
                        return Ok(false);
                    }
                    let mut file = PathBuf::from(&folder.path);
                    for part in parts {
                        file.push(part);
                    }
                    library.update_file(
                        &js::normalize(&file).to_string_lossy(),
                        FileChange {
                            star,
                            tags: tags.map(|tags| clean_tags(&json!(tags))),
                            note: note.map(|note| js::slice(js::trim(&note), 2000).to_string()),
                        },
                    );
                    return Ok(false);
                }
            }
            Ok(true)
        })();
        match result {
            Ok(false) => {}
            Ok(true) => self.send_library_views(),
            Err(error) => {
                self.toast(Some(client), &error_text(&error), Tone::Error);
                self.send_library_views();
            }
        }
    }

    // --- Workspaces and series ---

    /// Runs f with the collections and the app as their host, then carries out what it changed.
    fn with_collections<R>(
        &mut self,
        f: impl FnOnce(&mut Collections, &mut CollectionHost<'_>) -> R,
    ) -> R {
        let app = self.app;
        let State { hub, collections } = &mut **self.guard;
        let mut host = CollectionHost {
            hub,
            store: &*app.inner.options.store,
            removed: Vec::new(),
            changed: false,
        };
        let result = f(collections, &mut host);
        let (removed, changed) = (host.removed, host.changed);
        for project_id in removed {
            self.remove_project(&project_id);
        }
        if changed {
            let projects: Vec<SavedProject> = self
                .guard
                .hub
                .projects
                .values()
                .map(|project| project.saved.clone())
                .collect();
            for saved in &projects {
                self.prepare_folder(saved);
            }
            self.send_meta();
        }
        result
    }

    /// A workspace's or series' setup thread: its newest one, in a project made for it on first use.
    fn open_setup(&mut self, client: ClientId, collection_id: &str) {
        let Some(collection) = self.guard.collections.get(collection_id).cloned() else {
            return;
        };
        let existing = self
            .guard
            .hub
            .projects
            .values()
            .find(|project| {
                project.saved.setup
                    && project
                        .saved
                        .collection
                        .as_ref()
                        .is_some_and(|own| own.id == collection_id)
            })
            .map(|project| project.saved.id.clone());
        let project_id = existing.unwrap_or_else(|| {
            let created = self.options().store.create(NewProject {
                name: Some(format!("{} setup", collection.name)),
                link: None,
                local: LocalLink::default(),
                collection: Some(ProjectCollection {
                    id: collection.id.clone(),
                    name: collection.name.clone(),
                }),
                setup: true,
            });
            self.add_project(created, Vec::new())
        });
        let thread = self
            .project_threads(&project_id)
            .into_iter()
            .find(|id| !self.guard.hub.threads[id].thread.archived())
            .unwrap_or_else(|| self.new_thread(&project_id));
        self.select(client, Some(thread));
    }

    fn handle_collection_message(&mut self, client: ClientId, message: &CollectionMessage) {
        match self.with_collections(|collections, host| collections.handle(host, message)) {
            Ok(Some(refusal)) => self.toast(Some(client), &refusal, Tone::Error),
            Ok(None) => {}
            Err(error) => tracing::error!("Bad client message: {error:#}"),
        }
    }

    // --- Messages ---

    fn handle_message(&mut self, app: &App, client: ClientId, message: ClientMessage) {
        let current = self
            .guard
            .hub
            .clients
            .get(&client)
            .and_then(|view| view.thread_id.clone())
            .filter(|id| self.guard.hub.threads.contains_key(id));
        let current_project = current
            .as_ref()
            .map(|id| self.guard.hub.threads[id].project_id.clone());
        let settings = self.options().settings.clone();
        match message {
            ClientMessage::Library(message) => self.library_message(client, message),
            ClientMessage::Send {
                text,
                compact_first,
            } => {
                let text = js::trim(&text);
                if let (false, Some(current)) = (text.is_empty(), &current) {
                    self.start_turn(current, Some(text.to_string()), compact_first == Some(true));
                }
            }
            ClientMessage::Compact => {
                if let Some(current) = &current {
                    self.start_turn(current, None, false);
                }
            }
            ClientMessage::Stop => {
                if let Some(current) = &current {
                    self.with_thread(current, |thread, out| thread.stop(out));
                }
            }
            ClientMessage::SetProvider { provider } => {
                self.guard.hub.last_provider = provider;
                if let Some(settings) = &settings {
                    settings.save(Settings {
                        default_provider: Some(provider),
                        ..Settings::default()
                    });
                }
                if let Some(current) = &current {
                    let entry = &self.guard.hub.threads[current];
                    if entry.thread.provider() != provider {
                        let saved = self.guard.hub.projects[&entry.project_id].saved.clone();
                        let session = app.session_for(&saved, provider, None);
                        self.with_thread(current, |thread, out| {
                            thread.set_provider(provider, session, out);
                        });
                    }
                }
                self.send_meta();
            }
            ClientMessage::SetDefaultProvider { provider } => {
                self.guard.hub.last_provider = provider;
                if let Some(settings) = &settings {
                    settings.save(Settings {
                        default_provider: Some(provider),
                        ..Settings::default()
                    });
                }
                self.send_meta();
            }
            ClientMessage::SetApprovalMode { mode } => {
                *app.inner.approval_mode.write().unwrap() = mode;
                if let Some(settings) = &settings {
                    settings.save(Settings {
                        approval_mode: Some(mode),
                        ..Settings::default()
                    });
                }
                // Changes to the brief keep waiting: those ask in full access too.
                if mode == ApprovalMode::Auto {
                    let ids: Vec<String> = self.guard.hub.threads.keys().cloned().collect();
                    for id in ids {
                        if let Some(entry) = self.guard.hub.threads.get_mut(&id) {
                            entry.thread.settle_all_approvals(true, |request| {
                                !is_brief_tool(request.tool.split("__").last().unwrap_or(""))
                            });
                        }
                    }
                }
                self.send_meta();
            }
            ClientMessage::Approve { id, allow } => {
                let ids: Vec<String> = self.guard.hub.threads.keys().cloned().collect();
                for thread_id in ids {
                    if self.with_thread(&thread_id, |thread, out| thread.approve(&id, allow, out))
                        == Some(true)
                    {
                        break;
                    }
                }
            }
            ClientMessage::RefreshResolve => drop(app.refresh_resolve()),
            ClientMessage::LoadHistory => {
                if let Some(project_id) = current_project {
                    let app = app.clone();
                    tokio::spawn(
                        async move { app.publish_history(&project_id, Some(client)).await },
                    );
                }
            }
            ClientMessage::LoadProjectFile => {
                if let Some(project_id) = &current_project {
                    self.publish_project_file(project_id, Some(client));
                }
            }
            ClientMessage::SaveProjectFile { project_id, text } => {
                self.save_project_file(client, &project_id, &text);
            }
            ClientMessage::RenderPreview { timeline_id } => {
                self.render_preview(client, current_project, timeline_id);
            }
            ClientMessage::ApplyTimeline {
                project_id,
                timeline_id,
            } => {
                if let Some(applying) = self.begin_apply(client, &project_id, &timeline_id) {
                    let app = app.clone();
                    tokio::spawn(async move {
                        app.apply_timeline(client, project_id, timeline_id, applying)
                            .await;
                    });
                }
            }
            ClientMessage::StopPreview => {
                if let Some(job) = &self.guard.hub.preview_job {
                    job.stop();
                }
            }
            ClientMessage::SelectThread { thread_id } => {
                if self.guard.hub.threads.contains_key(&thread_id) {
                    self.select(client, Some(thread_id));
                }
            }
            ClientMessage::NewThread { project_id } => {
                if self.guard.hub.projects.contains_key(&project_id) {
                    let thread = self.new_thread(&project_id);
                    self.select(client, Some(thread));
                }
            }
            ClientMessage::RenameThread { thread_id, title } => {
                self.with_thread(&thread_id, |thread, out| thread.rename(&title, out));
            }
            ClientMessage::ArchiveThread {
                thread_id,
                archived,
            } => {
                let Some(entry) = self.guard.hub.threads.get(&thread_id) else {
                    return;
                };
                let project_id = entry.project_id.clone();
                if archived && entry.thread.empty() {
                    self.remove_thread(&thread_id);
                } else {
                    self.with_thread(&thread_id, |thread, out| thread.set_archived(archived, out));
                    if archived {
                        self.move_viewers(&thread_id, &project_id);
                    }
                }
            }
            ClientMessage::DeleteThread { thread_id } => self.remove_thread(&thread_id),
            ClientMessage::AddProject { source } => self.add_project_from(client, source),
            ClientMessage::UpdateProject {
                project_id,
                name,
                changes,
            } => {
                self.update_project(&project_id, name.as_deref());
                self.update_project_sidebar(&project_id, &changes);
            }
            ClientMessage::RelinkProject { project_id } => self.relink(client, &project_id),
            ClientMessage::RemoveProject { project_id } => self.remove_project(&project_id),
            ClientMessage::OpenInResolve { project_id } => {
                if let Some((link, local)) = self.begin_open(client, &project_id) {
                    let app = app.clone();
                    tokio::spawn(async move {
                        app.open_in_resolve(client, project_id, link, local).await;
                    });
                }
            }
            ClientMessage::LoadLibrary => {
                let app = app.clone();
                tokio::spawn(async move { app.load_library(client).await });
            }
            ClientMessage::BrowseFolders { request_id, path } => {
                let app = app.clone();
                tokio::spawn(async move { app.send_folders(client, request_id, path).await });
            }
            ClientMessage::RefreshRemoteAccess => {
                if let Some(remote) = self.options().remote.clone() {
                    tokio::spawn(async move { remote.refresh().await });
                }
            }
            ClientMessage::SetRemoteAccess { enabled } => {
                if let Some(remote) = self.options().remote.clone() {
                    tokio::spawn(async move { remote.set_enabled(enabled).await });
                }
            }
            ClientMessage::RefreshConnect => {
                if let Some(connect) = self.options().connect.clone() {
                    tokio::spawn(async move { connect.refresh().await });
                }
            }
            ClientMessage::SetConnect { enabled } => {
                if let Some(connect) = self.options().connect.clone() {
                    tokio::spawn(async move { connect.set_enabled(enabled).await });
                }
            }
            ClientMessage::ConnectUnlink { computer_id } => {
                if let Some(connect) = self.options().connect.clone()
                    && !computer_id.is_empty()
                {
                    tokio::spawn(async move { connect.unlink(&computer_id).await });
                }
            }
            ClientMessage::ConnectSignOutDevice { device_id } => {
                if let Some(connect) = &self.options().connect {
                    connect.sign_out_device(&device_id);
                }
            }
            ClientMessage::LinkerCancelSignIn => {
                if let Some(linker) = &self.options().linker {
                    linker.cancel();
                }
            }
            ClientMessage::LinkerSignOut => {
                if let Some(linker) = self.options().linker.clone() {
                    tokio::spawn(async move { linker.sign_out().await });
                }
            }
            ClientMessage::SetResolveHere { resolve_here } => self.set_resolve_here(resolve_here),
            ClientMessage::TranscriptionSetup => {
                if let Some(transcriber) = &self.options().transcriber {
                    let setup = transcriber.setup();
                    let app = app.clone();
                    tokio::spawn(async move {
                        let (text, tone) = match setup.await {
                            Ok(()) => ("Transcription is set up.".to_string(), Tone::Info),
                            Err(error) => (
                                format!("Couldn't set up transcription: {}", error_text(&error)),
                                Tone::Error,
                            ),
                        };
                        app.lock().toast(Some(client), &text, tone);
                    });
                }
            }
            ClientMessage::TranscriptionCancel => {
                if let Some(transcriber) = &self.options().transcriber {
                    transcriber.cancel_setup();
                }
            }
            ClientMessage::CheckAppUpdate => {
                if let Some(updates) = &self.options().app_updates {
                    updates.check();
                }
            }
            ClientMessage::InstallAppUpdate => {
                if let Some(updates) = &self.options().app_updates {
                    updates.install();
                }
            }
            ClientMessage::CancelAppUpdate => {
                if let Some(updates) = &self.options().app_updates {
                    updates.cancel();
                }
            }
            ClientMessage::Collection(CollectionMessage::SetupCollection { id }) => {
                self.open_setup(client, &id);
            }
            ClientMessage::Collection(message) => self.handle_collection_message(client, &message),
            ClientMessage::Sidebar(message) => self.handle_sidebar_message(message),
        }
    }

    // --- Sidebar (T3 Code) ---

    fn handle_sidebar_message(&mut self, message: SidebarClientMessage) {
        match message {
            SidebarClientMessage::PinThread { thread_id, pinned } => {
                self.with_thread(&thread_id, |thread, out| thread.set_pinned(pinned, out));
            }
            SidebarClientMessage::SettleThread { thread_id, settled } => {
                self.with_thread(&thread_id, |thread, out| thread.set_settled(settled, out));
            }
            SidebarClientMessage::SnoozeThread { thread_id, until } => match until {
                None => {
                    self.with_thread(&thread_id, |thread, out| thread.snooze(None, out));
                }
                Some(until) => {
                    if let Some(at) = crate::thread::parse_ms(&until)
                        && at > js::now_millis()
                    {
                        let until = js::iso_from_ms(at);
                        self.with_thread(&thread_id, |thread, out| thread.snooze(Some(until), out));
                    }
                }
            },
            SidebarClientMessage::MarkRead { thread_id, read } => {
                self.with_thread(&thread_id, |thread, out| thread.mark_read(read, out));
            }
        }
    }

    fn update_project_sidebar(&mut self, project_id: &str, changes: &ProjectSidebarChanges) {
        let Some(project) = self.guard.hub.projects.get_mut(project_id) else {
            return;
        };
        if changes.icon.is_none() && changes.pinned.is_none() && changes.archived.is_none() {
            return;
        }
        let before = project.saved.icon.clone();
        if let Some(icon) = &changes.icon {
            project.saved.icon = icon.clone();
        }
        if let Some(pinned) = changes.pinned {
            project.saved.pinned = pinned;
        }
        if let Some(archived) = changes.archived {
            project.saved.archived = archived;
        }
        self.app.inner.options.store.save_project(&project.saved);
        if project.saved.icon != before {
            commit_project_file(project.history.clone(), "Change project icon".into());
        }
        self.send_meta();
    }
}

#[cfg(test)]
#[path = "app/testing.rs"]
pub(crate) mod testing;

#[cfg(test)]
#[path = "app/tests.rs"]
mod tests;

// brief.test.ts, library/app.test.ts and the app part of collections.test.ts.
#[cfg(test)]
#[path = "app/brief_tests.rs"]
mod brief_tests;
#[cfg(test)]
#[path = "app/collections_tests.rs"]
mod collections_tests;
#[cfg(test)]
#[path = "app/library_tests.rs"]
mod library_tests;
