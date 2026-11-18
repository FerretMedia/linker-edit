// Each test module uses some of these.
#![allow(dead_code)]

// What the app's tests share: a client that keeps state as the UI does, agent sessions that run scripted turns,
// a history that records what it was asked to save, and settings in memory. The TypeScript's app.test.ts setup.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use linker_edit_protocol::{
    AppState, ChatItem, ClientMessage, HistoryEntry, LibraryView, PlayableTimeline, Provider,
    ResolveDatabase, ResolveStatus, ServerMessage,
};
use serde_json::Value;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::{App, AppOptions, BoxFuture, Client, ClientId, SessionRequest, create_app};
use crate::history::history::{AgentTurn, History, MediaFile, Saved};
use crate::projects::store::{LoadedProject, NEW_THREAD_SIDEBAR, SavedThread};
use crate::projects::testing::{MemoryProjectStore, memory_project_store, workspace_project};
use crate::providers::types::{ProviderSession, TurnSink};
use crate::settings::{Settings, SettingsStore};

/// applyServerMessage (src/lib/appState.ts): folds one server message into the client's copy of the state.
pub fn apply_server_message(state: &mut Option<AppState>, message: &ServerMessage) {
    match message {
        ServerMessage::Snapshot { state: snapshot } => *state = Some((**snapshot).clone()),
        ServerMessage::Meta { meta } => {
            if let Some(state) = state {
                state.meta = (**meta).clone();
            }
        }
        // An item of a thread the client just left can still be on its way.
        ServerMessage::Item { thread_id, item } => {
            let Some(state) = state else { return };
            if state.meta.thread_id.as_deref() != Some(thread_id.as_str()) {
                return;
            }
            let id = item_id(item);
            match state.items.iter().position(|other| item_id(other) == id) {
                Some(index) => state.items[index] = item.clone(),
                None => state.items.push(item.clone()),
            }
        }
        _ => {}
    }
}

pub fn item_id(item: &ChatItem) -> &str {
    match item {
        ChatItem::User { id, .. }
        | ChatItem::Assistant { id, .. }
        | ChatItem::Tool { id, .. }
        | ChatItem::Notice { id, .. } => id,
    }
}

/// A project file the app sent: (project id, file, text).
pub type ProjectFileMessage = (String, String, String);

/// A client that keeps state the way the UI does, and wakes waits as messages arrive.
pub struct TestClient {
    state: Mutex<Option<AppState>>,
    pub histories: Mutex<Vec<Vec<HistoryEntry>>>,
    pub refused: Mutex<Vec<String>>,
    pub toasts: Mutex<Vec<String>>,
    pub files: Mutex<Vec<ProjectFileMessage>>,
    pub views: Mutex<Vec<LibraryView>>,
    messages: watch::Sender<u64>,
}

impl Default for TestClient {
    fn default() -> Self {
        TestClient {
            state: Mutex::default(),
            histories: Mutex::default(),
            refused: Mutex::default(),
            toasts: Mutex::default(),
            files: Mutex::default(),
            views: Mutex::default(),
            messages: watch::channel(0).0,
        }
    }
}

impl Client for TestClient {
    fn send(&self, data: String) {
        let message: ServerMessage = serde_json::from_str(&data).unwrap_or_else(|error| {
            panic!("the app sent what the protocol doesn't have ({error}): {data}")
        });
        match &message {
            ServerMessage::History { entries, .. } => {
                self.histories.lock().unwrap().push(entries.clone());
            }
            ServerMessage::Refused { text, .. } => self.refused.lock().unwrap().push(text.clone()),
            ServerMessage::Toast { text, .. } => self.toasts.lock().unwrap().push(text.clone()),
            ServerMessage::ProjectFile {
                project_id,
                file,
                text,
            } => self
                .files
                .lock()
                .unwrap()
                .push((project_id.clone(), file.clone(), text.clone())),
            ServerMessage::LibraryView(
                linker_edit_protocol::LibraryServerMessage::LibraryView { view },
            ) => self.views.lock().unwrap().push(view.clone()),
            _ => {}
        }
        apply_server_message(&mut self.state.lock().unwrap(), &message);
        self.messages.send_modify(|count| *count += 1);
    }
}

impl TestClient {
    pub fn state(&self) -> AppState {
        self.state.lock().unwrap().clone().expect("No snapshot yet")
    }

    pub fn toasts(&self) -> Vec<String> {
        self.toasts.lock().unwrap().clone()
    }

    pub fn refused(&self) -> Vec<String> {
        self.refused.lock().unwrap().clone()
    }

    pub fn view(&self) -> LibraryView {
        self.views
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("no library view yet")
    }

    /// Waits until a message leaves the state as predicate wants it; at most 10 seconds, then fails.
    pub async fn until(&self, predicate: impl Fn(&AppState) -> bool) {
        self.until_checked(|state| state.is_some_and(&predicate))
            .await;
    }

    /// Waits until check is true, checked again after each message.
    pub async fn until_checked(&self, check: impl Fn(Option<&AppState>) -> bool) {
        let mut messages = self.messages.subscribe();
        let wait = async {
            loop {
                if check(self.state.lock().unwrap().as_ref()) {
                    return;
                }
                messages.changed().await.expect("the client is kept");
            }
        };
        tokio::time::timeout(Duration::from_secs(10), wait)
            .await
            .expect("waited 10 seconds for the app");
    }
}

/// What a scripted turn gets: the turn's sink and signal, and the app, to call the agents' tools as an MCP
/// server would.
pub struct Turn {
    pub sink: Arc<dyn TurnSink>,
    pub signal: CancellationToken,
    pub app: App,
}

pub type Script = Box<dyn FnOnce(Turn) -> BoxFuture<anyhow::Result<()>> + Send>;

pub fn script<F, Fut>(run: F) -> Script
where
    F: FnOnce(Turn) -> Fut + Send + 'static,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    Box::new(move |turn| Box::pin(run(turn)))
}

/// A turn that does nothing.
pub fn quiet() -> Script {
    script(|_| async { Ok(()) })
}

/// What the sessions saw, shared by every session of a test app.
#[derive(Default)]
pub struct Sessions {
    pub providers: Mutex<Vec<Provider>>,
    pub resume_ids: Mutex<Vec<Option<String>>>,
    pub prompts: Mutex<Vec<String>>,
    pub dirs: Mutex<Vec<String>>,
    /// The project's rules as each turn read them.
    pub rules: Mutex<Vec<String>>,
    scripts: Mutex<VecDeque<Script>>,
    app: OnceLock<App>,
}

/// Sessions get a provider id on their first send, like the real ones. Claude's compact() sends "/compact" and
/// runs the next script, so compactions read top to bottom with the turns.
struct FakeSession {
    provider: Provider,
    id: Mutex<Option<String>>,
    name: String,
    rules: Arc<dyn Fn() -> String + Send + Sync>,
    shared: Arc<Sessions>,
}

#[async_trait]
impl ProviderSession for FakeSession {
    async fn send(
        &self,
        text: &str,
        sink: Arc<dyn TurnSink>,
        signal: CancellationToken,
    ) -> anyhow::Result<()> {
        self.shared.prompts.lock().unwrap().push(text.to_string());
        self.shared.rules.lock().unwrap().push((self.rules)());
        self.id.lock().unwrap().get_or_insert(self.name.clone());
        let next = self.shared.scripts.lock().unwrap().pop_front();
        let Some(next) = next else {
            anyhow::bail!("No script left for this send");
        };
        let app = self.shared.app.get().expect("the app is made").clone();
        next(Turn { sink, signal, app }).await
    }

    fn can_compact(&self) -> bool {
        self.provider == Provider::Claude
    }

    async fn compact(
        &self,
        sink: Arc<dyn TurnSink>,
        signal: CancellationToken,
    ) -> anyhow::Result<()> {
        self.send("/compact", sink, signal).await
    }

    fn resume_id(&self) -> Option<String> {
        self.id.lock().unwrap().clone()
    }
}

/// A history that records what it was asked to save.
#[derive(Default)]
pub struct FakeHistory {
    pub records: Mutex<Vec<Option<AgentTurn>>>,
    pub commits: Mutex<Vec<String>>,
    /// What a save of an agent's turn changed.
    pub changes: Vec<String>,
    pub recent: String,
    /// The log: given how many saves were recorded so far.
    pub log: Option<Box<dyn Fn(usize) -> Vec<HistoryEntry> + Send + Sync>>,
    pub timeline: Option<PlayableTimeline>,
}

impl FakeHistory {
    pub fn records(&self) -> Vec<Option<AgentTurn>> {
        self.records.lock().unwrap().clone()
    }

    pub fn commits(&self) -> Vec<String> {
        self.commits.lock().unwrap().clone()
    }
}

#[async_trait]
impl History for FakeHistory {
    async fn record(&self, turn: Option<AgentTurn>) -> anyhow::Result<Option<Saved>> {
        let saved = turn.is_some().then(|| Saved {
            commit: "abc1234".into(),
            changes: self.changes.clone(),
        });
        self.records.lock().unwrap().push(turn);
        Ok(saved)
    }

    async fn recent_context(&self) -> anyhow::Result<String> {
        Ok(self.recent.clone())
    }

    async fn log(&self, _: Option<usize>) -> anyhow::Result<Vec<HistoryEntry>> {
        let count = self.records.lock().unwrap().len();
        Ok(self.log.as_ref().map(|log| log(count)).unwrap_or_default())
    }

    async fn timeline(
        &self,
        _: Option<String>,
        _: Option<String>,
    ) -> anyhow::Result<Option<PlayableTimeline>> {
        Ok(self.timeline.clone())
    }

    fn media_file(&self, _: &str) -> Option<MediaFile> {
        None
    }

    async fn commit_project_file(&self, message: &str) -> anyhow::Result<()> {
        self.commits.lock().unwrap().push(message.to_string());
        Ok(())
    }
}

/// Settings kept in memory: what one app saves, the next one loads.
#[derive(Default)]
pub struct MemorySettings(pub Mutex<Settings>);

impl SettingsStore for MemorySettings {
    fn load(&self) -> Settings {
        self.0.lock().unwrap().clone()
    }

    fn save(&self, changes: Settings) {
        let mut saved = self.0.lock().unwrap();
        if changes.approval_mode.is_some() {
            saved.approval_mode = changes.approval_mode;
        }
        if changes.resolve_here.is_some() {
            saved.resolve_here = changes.resolve_here;
        }
        if changes.default_provider.is_some() {
            saved.default_provider = changes.default_provider;
        }
    }
}

impl MemorySettings {
    pub fn saved(&self) -> Settings {
        self.0.lock().unwrap().clone()
    }
}

pub fn connected(project: Option<&str>, project_id: Option<&str>) -> ResolveStatus {
    ResolveStatus::Connected {
        product: "DaVinci Resolve Studio".into(),
        version: "21".into(),
        project: project.map(str::to_string),
        project_id: project_id.map(str::to_string),
        database: Some(ResolveDatabase {
            r#type: "Disk".into(),
            name: "Local Database".into(),
            host: None,
        }),
        timeline: None,
    }
}

/// Resolve's status, changed by a test as the user switches projects in Resolve.
pub fn resolve_with(status: ResolveStatus) -> (Arc<Mutex<ResolveStatus>>, super::CheckResolve) {
    let current = Arc::new(Mutex::new(status));
    let check = current.clone();
    let check: super::CheckResolve = Arc::new(move || {
        let status = check.lock().unwrap().clone();
        Box::pin(async move { status })
    });
    (current, check)
}

pub fn saved_thread() -> SavedThread {
    SavedThread {
        id: "thread-1".into(),
        title: "cut it".into(),
        provider: Provider::Claude,
        resume_id: None,
        items: Vec::new(),
        context: None,
        created_at: "2026-11-09T10:00:00.000Z".into(),
        updated_at: "2026-11-09T10:00:00.000Z".into(),
        archived_at: None,
        sidebar: NEW_THREAD_SIDEBAR,
    }
}

/// A message as the UI sends it; one the protocol doesn't have is None, as the socket drops it.
pub fn message(value: Value) -> Option<ClientMessage> {
    serde_json::from_value(value).ok()
}

pub struct Harness {
    pub app: App,
    pub client: Arc<TestClient>,
    pub id: ClientId,
    pub sessions: Arc<Sessions>,
    pub store: Arc<MemoryProjectStore>,
}

impl Harness {
    /// Sends a message as the UI does, as JSON; one the protocol doesn't have is dropped.
    pub fn send(&self, value: Value) {
        if let Some(message) = message(value) {
            self.app.handle_message(self.id, message);
        }
    }

    pub fn state(&self) -> AppState {
        self.client.state()
    }

    pub async fn turn_done(&self) {
        self.client.until(|state| !state.meta.busy).await;
    }

    /// Every turn is over, including one in a thread the client moved away from or deleted.
    pub async fn all_done(&self) {
        let app = self.app.clone();
        self.client.until_checked(|_| !app.busy()).await;
    }

    pub fn disconnect(&self) {
        self.app.disconnect(self.id);
    }

    pub fn prompts(&self) -> Vec<String> {
        self.sessions.prompts.lock().unwrap().clone()
    }

    pub fn rules(&self) -> Vec<String> {
        self.sessions.rules.lock().unwrap().clone()
    }

    /// Another client, connected now.
    pub fn connect(&self) -> (Arc<TestClient>, ClientId) {
        let client = Arc::new(TestClient::default());
        let id = self.app.connect(client.clone());
        (client, id)
    }
}

/// An app on one project, whose sessions run the scripts in order across sessions, so tests read top to bottom.
pub struct Setup {
    pub store: Option<Arc<MemoryProjectStore>>,
    pub history: Option<Arc<FakeHistory>>,
    pub scripts: Vec<Script>,
    configure: Option<Configure>,
}

type Configure = Box<dyn FnOnce(&mut AppOptions)>;

pub fn setup() -> Setup {
    Setup {
        store: None,
        history: None,
        scripts: Vec::new(),
        configure: None,
    }
}

impl Setup {
    pub fn store(mut self, store: Arc<MemoryProjectStore>) -> Self {
        self.store = Some(store);
        self
    }

    pub fn history(mut self, history: Arc<FakeHistory>) -> Self {
        self.history = Some(history);
        self
    }

    pub fn scripts(mut self, scripts: impl IntoIterator<Item = Script>) -> Self {
        self.scripts.extend(scripts);
        self
    }

    pub fn with(mut self, configure: impl FnOnce(&mut AppOptions) + 'static) -> Self {
        self.configure = Some(Box::new(configure));
        self
    }

    pub fn start(self) -> Harness {
        let store = self.store.unwrap_or_else(|| {
            Arc::new(memory_project_store(vec![LoadedProject {
                project: workspace_project(),
                threads: Vec::new(),
            }]))
        });
        let sessions = Arc::new(Sessions::default());
        sessions.scripts.lock().unwrap().extend(self.scripts);
        let shared = sessions.clone();
        let create_session: super::CreateSession = Arc::new(move |request: SessionRequest| {
            let SessionRequest {
                provider,
                dir,
                resume_id,
                rules,
                ..
            } = request;
            let mut providers = shared.providers.lock().unwrap();
            providers.push(provider);
            shared.resume_ids.lock().unwrap().push(resume_id.clone());
            shared
                .dirs
                .lock()
                .unwrap()
                .push(dir.to_string_lossy().into_owned());
            Arc::new(FakeSession {
                provider,
                id: Mutex::new(resume_id),
                name: format!(
                    "{}-{}",
                    match provider {
                        Provider::Claude => "claude",
                        Provider::Codex => "codex",
                    },
                    providers.len()
                ),
                rules,
                shared: shared.clone(),
            }) as Arc<dyn ProviderSession>
        });
        let mut options = AppOptions::new(
            "0.0.0",
            store.clone(),
            create_session,
            Arc::new(|| Box::pin(async { ResolveStatus::NotRunning })),
        );
        if let Some(history) = self.history {
            options.create_history =
                Some(Arc::new(move |_, _| history.clone() as Arc<dyn History>));
        }
        if let Some(configure) = self.configure {
            configure(&mut options);
        }
        let app = create_app(options);
        let _ = sessions.app.set(app.clone());
        let client = Arc::new(TestClient::default());
        let id = app.connect(client.clone());
        Harness {
            app,
            client,
            id,
            sessions,
            store,
        }
    }
}

/// The text of each user message, and the kind of every other item.
pub fn kinds(items: &[ChatItem]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            ChatItem::User { text, .. } => text.clone(),
            ChatItem::Assistant { .. } => "assistant".into(),
            ChatItem::Tool { .. } => "tool".into(),
            ChatItem::Notice { .. } => "notice".into(),
        })
        .collect()
}

/// The notices: (tone, text).
pub fn notices(items: &[ChatItem]) -> Vec<(linker_edit_protocol::Tone, String)> {
    items
        .iter()
        .filter_map(|item| match item {
            ChatItem::Notice { text, tone, .. } => Some((*tone, text.clone())),
            _ => None,
        })
        .collect()
}
