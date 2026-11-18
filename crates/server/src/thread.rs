// One conversation with an agent: its chat, its provider session and its turn. The port of server/thread.ts.
//
// A Thread lives inside the app's state (app.rs), behind its lock. Its methods change it and say what the app
// must do about it as ThreadEvents (the TypeScript's host calls), which the app carries out while it still holds
// the lock. A turn's waiting (on the agent, on Resolve, on the history) happens in run_turn, outside the lock.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use indexmap::IndexMap;
use linker_edit_protocol::{
    ApprovalRequest, ChatItem, ContextUsage, Provider, ThreadSummary, Tone, ToolStatus,
};
use serde_json::Value;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::app::TurnContext;
use crate::handoff::{BUDGET_CHARS, handoff_summary, with_handoff};
use crate::history::history::{AgentTurn, History, resolve_write_action, with_history_context};
use crate::js;
use crate::policy::is_resolve_write;
use crate::projects::store::{SavedThread, ThreadSidebar, thread_title};
use crate::providers::types::{ItemKind, ItemPatch, ProviderSession, TurnSink};

pub const DEFAULT_TITLE: &str = "New thread";

pub fn provider_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "Claude",
        Provider::Codex => "Codex",
    }
}

/// What the app does after a thread changed: the TypeScript's ThreadHost calls.
#[derive(Debug, Clone, PartialEq)]
pub enum ThreadEvent {
    /// Anything in the thread's meta or summary changed.
    Changed,
    Item(ChatItem),
    /// The message didn't go to the agent; the client puts it back.
    Refused {
        text: String,
        reason: String,
    },
    Persist(Box<SavedThread>),
}

pub type Events = Vec<ThreadEvent>;

struct Approval {
    request: ApprovalRequest,
    settle: oneshot::Sender<bool>,
}

/// Out of Settled and Snoozed.
fn unparked(sidebar: &mut ThreadSidebar, now: &str) {
    sidebar.snoozed_until = None;
    sidebar.snoozed_at = None;
    if sidebar.settled_at.is_some() {
        sidebar.settled_at = None;
        sidebar.unsettled_at = Some(now.to_string());
    }
}

/// Date.parse for the ISO 8601 dates the app writes and clients send; None when it isn't one.
pub fn parse_ms(text: &str) -> Option<i64> {
    if let Ok(date) = chrono::DateTime::parse_from_rfc3339(text) {
        return Some(date.timestamp_millis());
    }
    chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|date| date.and_utc().timestamp_millis())
}

/// `{ ...existing, ...patch } as ChatItem`; None when the result isn't a chat item.
fn merged(existing: Option<&ChatItem>, patch: &ItemPatch) -> Option<ChatItem> {
    let mut object = existing
        .and_then(|item| serde_json::to_value(item).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    if let Ok(Value::Object(fields)) = serde_json::to_value(patch) {
        object.extend(fields);
    }
    serde_json::from_value(Value::Object(object)).ok()
}

fn item_id(item: &ChatItem) -> &str {
    match item {
        ChatItem::User { id, .. }
        | ChatItem::Assistant { id, .. }
        | ChatItem::Tool { id, .. }
        | ChatItem::Notice { id, .. } => id,
    }
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn notice(text: impl Into<String>, tone: Tone) -> ItemPatch {
    ItemPatch {
        id: new_id(),
        kind: Some(ItemKind::Notice),
        text: Some(text.into()),
        tone: Some(tone),
        ..ItemPatch::default()
    }
}

pub struct Thread {
    pub id: String,
    title: String,
    provider: Provider,
    items: Vec<ChatItem>,
    context: Option<ContextUsage>,
    created_at: String,
    updated_at: String,
    archived_at: Option<String>,
    // Where the sidebar shows it: pinned, settled, snoozed, unread. See ThreadSidebarState.
    sidebar: ThreadSidebar,
    turn: Option<CancellationToken>,
    // The running turn's sink, for approvals asked outside the provider session, as by the Resolve guard.
    active_sink: Option<Arc<ThreadSink>>,
    // Set once the thread is deleted, so a turn that is still winding down can't write into it.
    disposed: bool,
    approvals: IndexMap<String, Approval>,
    session: Arc<dyn ProviderSession>,
    // Until a turn of it succeeds: the provider may have dropped the conversation since.
    resumed: bool,
}

impl Thread {
    /// session is the provider session for the saved provider, continuing saved.resume_id when there is one.
    pub fn new(saved: SavedThread, session: Arc<dyn ProviderSession>) -> Self {
        Thread {
            resumed: saved.resume_id.is_some(),
            id: saved.id,
            title: saved.title,
            provider: saved.provider,
            items: saved.items,
            context: saved.context,
            created_at: saved.created_at,
            updated_at: saved.updated_at,
            archived_at: saved.archived_at,
            sidebar: saved.sidebar,
            turn: None,
            active_sink: None,
            disposed: false,
            approvals: IndexMap::new(),
            session,
        }
    }

    pub fn items(&self) -> &[ChatItem] {
        &self.items
    }

    pub fn provider(&self) -> Provider {
        self.provider
    }

    pub fn context(&self) -> Option<&ContextUsage> {
        self.context.as_ref()
    }

    pub fn busy(&self) -> bool {
        self.turn.is_some()
    }

    /// Cancelled when the running turn is stopped; None between turns.
    pub fn signal(&self) -> Option<CancellationToken> {
        self.turn.clone()
    }

    pub fn archived(&self) -> bool {
        self.archived_at.is_some()
    }

    pub fn updated_at(&self) -> &str {
        &self.updated_at
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn disposed(&self) -> bool {
        self.disposed
    }

    pub(crate) fn active_sink(&self) -> Option<Arc<ThreadSink>> {
        self.active_sink.clone()
    }

    pub fn approvals(&self) -> Vec<ApprovalRequest> {
        self.approvals
            .values()
            .map(|entry| entry.request.clone())
            .collect()
    }

    pub fn summary(&self) -> ThreadSummary {
        ThreadSummary {
            id: self.id.clone(),
            title: self.title.clone(),
            updated_at: self.updated_at.clone(),
            archived: self.archived_at.is_some(),
            busy: self.turn.is_some(),
            waiting: !self.approvals.is_empty(),
            sidebar: self.sidebar.with_created_at(&self.created_at),
        }
    }

    fn record(&self) -> SavedThread {
        SavedThread {
            id: self.id.clone(),
            title: self.title.clone(),
            provider: self.provider,
            resume_id: self.session.resume_id(),
            items: self.items.clone(),
            context: self.context.clone(),
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            archived_at: self.archived_at.clone(),
            sidebar: self.sidebar.clone(),
        }
    }

    // An empty thread is kept in memory only, so unused new threads don't pile up across restarts.
    fn save(&self, out: &mut Events) {
        if !self.disposed && !self.items.is_empty() {
            out.push(ThreadEvent::Persist(Box::new(self.record())));
        }
    }

    fn upsert(&mut self, patch: &ItemPatch, out: &mut Events) {
        let index = self.items.iter().position(|item| item_id(item) == patch.id);
        let Some(item) = merged(index.map(|index| &self.items[index]), patch) else {
            return;
        };
        match index {
            Some(index) => self.items[index] = item.clone(),
            None => self.items.push(item.clone()),
        }
        if matches!(item, ChatItem::User { .. }) && self.title == DEFAULT_TITLE {
            self.title = thread_title(&self.items);
        }
        out.push(ThreadEvent::Item(item));
    }

    /// Settles the waiting approvals, or only those that `only` picks.
    pub fn settle_all_approvals(&mut self, allow: bool, only: impl Fn(&ApprovalRequest) -> bool) {
        let ids: Vec<String> = self
            .approvals
            .iter()
            .filter(|(_, entry)| only(&entry.request))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if let Some(entry) = self.approvals.shift_remove(&id) {
                let _ = entry.settle.send(allow);
            }
        }
    }

    fn stop_turn(&mut self) {
        if let Some(turn) = &self.turn {
            turn.cancel();
        }
        self.settle_all_approvals(false, |_| true);
    }

    pub fn stop(&mut self, out: &mut Events) {
        self.stop_turn();
        out.push(ThreadEvent::Changed);
    }

    /// The chat goes on; the new agent gets a summary of it with the next message. session is a fresh one of the
    /// next provider: the caller makes it only when the provider changes.
    pub fn set_provider(
        &mut self,
        next: Provider,
        session: Arc<dyn ProviderSession>,
        out: &mut Events,
    ) {
        if next == self.provider {
            return;
        }
        self.stop_turn();
        self.provider = next;
        self.session = session;
        self.resumed = false;
        self.context = None;
        if self
            .items
            .iter()
            .any(|item| matches!(item, ChatItem::User { .. }))
        {
            let text = format!(
                "Switched to {}. It gets a summary of this chat with your next message.",
                provider_name(next)
            );
            self.upsert(&notice(text, Tone::Info), out);
        }
        out.push(ThreadEvent::Changed);
        self.save(out);
    }

    /// True when the request was this thread's.
    pub fn approve(&mut self, request_id: &str, allow: bool, out: &mut Events) -> bool {
        let Some(entry) = self.approvals.shift_remove(request_id) else {
            return false;
        };
        let _ = entry.settle.send(allow);
        out.push(ThreadEvent::Changed);
        true
    }

    /// A line in the chat from Linker-Edit itself, such as an approved edit.
    pub fn note(&mut self, text: &str, out: &mut Events) {
        if self.disposed {
            return;
        }
        self.upsert(&notice(text, Tone::Info), out);
        self.save(out);
    }

    pub fn rename(&mut self, next: &str, out: &mut Events) {
        let title = js::slice(js::trim(next), 120);
        if !title.is_empty() {
            self.title = title.to_string();
        }
        out.push(ThreadEvent::Changed);
        self.save(out);
    }

    pub fn set_archived(&mut self, archived: bool, out: &mut Events) {
        if archived {
            self.stop_turn();
        }
        self.archived_at = archived.then(js::now_iso);
        out.push(ThreadEvent::Changed);
        self.save(out);
    }

    /// Pinned threads sit above the projects.
    pub fn set_pinned(&mut self, pinned: bool, out: &mut Events) {
        let now = js::now_iso();
        if pinned {
            unparked(&mut self.sidebar, &now);
            self.sidebar.pinned_at.get_or_insert(now);
        } else {
            self.sidebar.pinned_at = None;
        }
        out.push(ThreadEvent::Changed);
        self.save(out);
    }

    /// Settling unpins and wakes it too; unsettling puts it back in its project.
    pub fn set_settled(&mut self, settled: bool, out: &mut Events) {
        let now = js::now_iso();
        if settled {
            let sidebar = &mut self.sidebar;
            sidebar.pinned_at = None;
            sidebar.snoozed_until = None;
            sidebar.snoozed_at = None;
            sidebar.settled_at.get_or_insert(now);
            sidebar.unsettled_at = None;
        } else {
            unparked(&mut self.sidebar, &now);
        }
        out.push(ThreadEvent::Changed);
        self.save(out);
    }

    /// Hides it until then; None wakes it now. A snoozed thread isn't settled.
    pub fn snooze(&mut self, until: Option<String>, out: &mut Events) {
        let now = js::now_iso();
        match until {
            None => {
                self.sidebar.snoozed_until = None;
                self.sidebar.snoozed_at = None;
            }
            Some(until) => {
                unparked(&mut self.sidebar, &now);
                self.sidebar.snoozed_until = Some(until);
                self.sidebar.snoozed_at = Some(now);
            }
        }
        out.push(ThreadEvent::Changed);
        self.save(out);
    }

    /// Read: seen now. Unread: its last turn shows as new again.
    pub fn mark_read(&mut self, read: bool, out: &mut Events) {
        let ended = self.sidebar.turn_ended_at.as_deref().and_then(parse_ms);
        let last_read_at = if read {
            Some(js::now_iso())
        } else {
            match ended {
                None => self.sidebar.last_read_at.clone(),
                Some(ended) => Some(js::iso_from_ms(ended - 1)),
            }
        };
        if last_read_at == self.sidebar.last_read_at {
            return;
        }
        self.sidebar.last_read_at = last_read_at;
        out.push(ThreadEvent::Changed);
        self.save(out);
    }

    /// Stops the turn; a turn still winding down saves its Resolve changes but writes nothing here.
    pub fn dispose(&mut self) {
        self.stop_turn();
        self.disposed = true;
    }

    /// Whether a turn may start: none is running and, for a compaction, the session can compact a conversation.
    pub(crate) fn can_start(&self, text: Option<&str>) -> bool {
        if self.turn.is_some() || self.disposed {
            return false;
        }
        text.is_some() || (self.session.can_compact() && self.session.resume_id().is_some())
    }

    /// Resolve was claimed for this thread's turn: it is busy from now on.
    pub(crate) fn begin(&mut self, out: &mut Events) -> CancellationToken {
        let token = CancellationToken::new();
        self.turn = Some(token.clone());
        out.push(ThreadEvent::Changed);
        token
    }

    pub(crate) fn refused(&mut self, text: Option<String>, reason: String, out: &mut Events) {
        if let Some(text) = text {
            out.push(ThreadEvent::Refused { text, reason });
        }
    }
}

/// A tool call of the turn: its name and input, as far as they arrived.
type ToolCall = (Option<String>, Option<Value>);

/// The sink a turn writes into. Kept outside the chat items, so a turn's Resolve writes are still saved if its
/// thread is deleted.
pub struct ThreadSink {
    host: Arc<TurnContext>,
    signal: CancellationToken,
    session: Arc<dyn ProviderSession>,
    produced: AtomicBool,
    tools: Mutex<IndexMap<String, ToolCall>>,
    reply: Mutex<String>,
    // One backup per turn, before its first change; a failed one is tried again before the next change.
    backed_up: tokio::sync::Mutex<bool>,
}

impl ThreadSink {
    fn live_update(&self, f: impl FnOnce(&mut Thread, &mut Events)) -> bool {
        self.host.with_thread(f).is_some()
    }

    fn actions(&self) -> Vec<String> {
        self.tools
            .lock()
            .unwrap()
            .values()
            .filter_map(|(name, input)| {
                resolve_write_action(
                    name.as_deref(),
                    input.as_ref().unwrap_or(&Value::Null),
                    is_resolve_write,
                )
            })
            .collect()
    }

    fn reply(&self) -> String {
        self.reply.lock().unwrap().clone()
    }
}

#[async_trait]
impl TurnSink for ThreadSink {
    fn upsert(&self, patch: ItemPatch) {
        self.produced.store(true, Ordering::SeqCst);
        {
            let mut tools = self.tools.lock().unwrap();
            if patch.kind == Some(ItemKind::Tool) || tools.contains_key(&patch.id) {
                let (name, input) = tools.get(&patch.id).cloned().unwrap_or_default();
                tools.insert(
                    patch.id.clone(),
                    (patch.name.clone().or(name), patch.input.clone().or(input)),
                );
            }
        }
        if patch.kind == Some(ItemKind::Assistant)
            && let Some(text) = patch.text.as_ref().filter(|text| !text.is_empty())
        {
            *self.reply.lock().unwrap() = text.clone();
        }
        self.live_update(|thread, out| thread.upsert(&patch, out));
    }

    fn notice(&self, text: String, tone: Tone) {
        let patch = notice(text, tone);
        self.live_update(|thread, out| thread.upsert(&patch, out));
    }

    async fn request_approval(&self, tool: &str, input: &Value) -> bool {
        if self.signal.is_cancelled() {
            return false;
        }
        let mut waiting = None;
        self.live_update(|thread, out| {
            let (settle, wait) = oneshot::channel();
            let id = new_id();
            let request = ApprovalRequest {
                id: id.clone(),
                tool: tool.to_string(),
                input: input.clone(),
            };
            thread.approvals.insert(id, Approval { request, settle });
            out.push(ThreadEvent::Changed);
            waiting = Some(wait);
        });
        match waiting {
            Some(wait) => wait.await.unwrap_or(false),
            None => false,
        }
    }

    fn context(&self, usage: Option<ContextUsage>) {
        self.live_update(|thread, out| {
            if Arc::ptr_eq(&thread.session, &self.session) {
                thread.context = usage.clone();
                out.push(ThreadEvent::Changed);
            }
        });
    }

    async fn backup(&self) -> Result<(), String> {
        let mut done = self.backed_up.lock().await;
        if *done {
            return Ok(());
        }
        match self.host.backup().await {
            Ok(file) => {
                *done = true;
                if let Some(file) = file {
                    self.notice(
                        format!("Backed up the Resolve project to {file} before changing it."),
                        Tone::Info,
                    );
                }
                Ok(())
            }
            Err(error) => Err(format!(
                "Linker-Edit backs the Resolve project up before every change and couldn't this time, so nothing was changed: {error}"
            )),
        }
    }
}

/// A new session knows nothing yet: it gets the edit history, and a summary of the chat when one is going.
async fn prime_new_session(
    history: Option<&Arc<dyn History>>,
    text: &str,
    earlier: &[ChatItem],
) -> String {
    let prompt = with_handoff(&handoff_summary(earlier, BUDGET_CHARS), text);
    let Some(history) = history else {
        return prompt;
    };
    let context = history.recent_context().await.unwrap_or_default();
    with_history_context(&context, &prompt)
}

async fn save_history(
    host: &TurnContext,
    sink: &ThreadSink,
    history: &Arc<dyn History>,
    turn: Option<AgentTurn>,
) {
    let by_agent = turn.is_some();
    match history.record(turn).await {
        Ok(Some(saved)) => {
            host.saved().await;
            let who = if by_agent {
                "Saved to edit history"
            } else if host.shared() {
                "Saved changes made in Resolve outside Linker-Edit to edit history"
            } else {
                "Saved your own Resolve changes to edit history"
            };
            let changes = if saved.changes.is_empty() {
                ".".to_string()
            } else {
                format!(": {}", saved.changes.join("; "))
            };
            sink.notice(format!("{who} ({}){changes}", saved.commit), Tone::Info);
        }
        Ok(None) => {}
        Err(error) => sink.notice(format!("Couldn't save edit history: {error}"), Tone::Error),
    }
}

/// The rest of a turn once Resolve was claimed for it (Thread::begin). text None only compacts the chat.
pub(crate) async fn run_turn(
    host: Arc<TurnContext>,
    text: Option<String>,
    compact_first: bool,
    signal: CancellationToken,
) {
    let checked = host.check().await;
    let notice_text = match checked {
        Err(refusal) => {
            host.with_thread(|thread, _| thread.turn = None);
            host.release();
            host.changed();
            if let Some(text) = text {
                host.with_thread(|thread, out| thread.refused(Some(text), refusal, out));
            }
            return;
        }
        Ok(notice) => notice,
    };

    let started = host.with_thread(|thread, out| {
        let turn_provider = thread.provider;
        let turn_session = thread.session.clone();
        let earlier = thread.items.clone();
        thread.updated_at = js::now_iso();
        // Writing in a settled or snoozed thread brings it back, as in T3 Code.
        if text.is_some() {
            let now = thread.updated_at.clone();
            unparked(&mut thread.sidebar, &now);
            thread.sidebar.last_sent_at = Some(now);
        }
        if let Some(text) = &text {
            let user = ItemPatch {
                id: new_id(),
                kind: Some(ItemKind::User),
                text: Some(text.clone()),
                ..ItemPatch::default()
            };
            thread.upsert(&user, out);
        }
        if let Some(notice_text) = &notice_text {
            thread.upsert(&notice(notice_text.clone(), Tone::Info), out);
        }
        out.push(ThreadEvent::Changed);
        // Saved now too, so a crash or a closed app mid-turn doesn't lose the message.
        thread.save(out);
        (turn_provider, turn_session, earlier)
    });
    // Deleted while Resolve was checked: it was stopped, so nothing runs.
    let Some((turn_provider, turn_session, earlier)) = started else {
        host.release();
        host.changed();
        host.refresh_resolve();
        return;
    };

    let sink = Arc::new(ThreadSink {
        host: host.clone(),
        signal: signal.clone(),
        session: turn_session.clone(),
        produced: AtomicBool::new(false),
        tools: Mutex::default(),
        reply: Mutex::default(),
        backed_up: tokio::sync::Mutex::new(false),
    });
    let dyn_sink: Arc<dyn TurnSink> = sink.clone();
    host.with_thread(|thread, _| thread.active_sink = Some(sink.clone()));

    let history = host.history();
    let result: anyhow::Result<()> = async {
        let Some(text) = &text else {
            return turn_session.compact(dyn_sink.clone(), signal.clone()).await;
        };
        if let Some(history) = &history
            && host.resolve_connected()
        {
            save_history(host.as_ref(), &sink, history, None).await;
        }
        if compact_first
            && turn_session.can_compact()
            && turn_session.resume_id().is_some()
            && !signal.is_cancelled()
        {
            // A failed compaction shouldn't cost the user their message.
            if let Err(error) = turn_session
                .compact(dyn_sink.clone(), signal.clone())
                .await
                && !signal.is_cancelled()
            {
                sink.notice(format!("Couldn't compact the chat: {error}"), Tone::Error);
            }
        }
        let prompt = if turn_session.resume_id().is_some() {
            text.clone()
        } else {
            prime_new_session(history.as_ref(), text, &earlier).await
        };
        let sent = if signal.is_cancelled() {
            Ok(())
        } else {
            turn_session
                .send(&prompt, dyn_sink.clone(), signal.clone())
                .await
        };
        if let Err(error) = sent {
            let retry = host.with_thread(|thread, _| {
                (thread.resumed && Arc::ptr_eq(&thread.session, &turn_session))
                    .then_some(thread.provider)
            });
            let Some(Some(provider)) = retry else {
                return Err(error);
            };
            if sink.produced.load(Ordering::SeqCst) || signal.is_cancelled() {
                return Err(error);
            }
            let session = host.create_session(provider);
            host.with_thread(|thread, _| {
                thread.session = session.clone();
                thread.resumed = false;
            });
            sink.notice(
                format!(
                    "Couldn't continue the earlier conversation ({error}), so a new one started with a summary of this chat."
                ),
                Tone::Info,
            );
            let prompt = prime_new_session(history.as_ref(), text, &earlier).await;
            session.send(&prompt, dyn_sink.clone(), signal.clone()).await?;
        }
        host.with_thread(|thread, _| {
            if Arc::ptr_eq(&thread.session, &turn_session) {
                thread.resumed = false;
            }
        });
        Ok(())
    }
    .await;
    if let Err(error) = result
        && !signal.is_cancelled()
    {
        sink.notice(error.to_string(), Tone::Error);
    }

    host.with_thread(|thread, _| thread.active_sink = None);
    if signal.is_cancelled() {
        sink.notice("Stopped.".into(), Tone::Info);
    }
    let running: Vec<String> = host
        .with_thread(|thread, _| {
            thread
                .items
                .iter()
                .filter_map(|item| match item {
                    ChatItem::Tool {
                        id,
                        status: ToolStatus::Running,
                        ..
                    } => Some(id.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    for id in running {
        sink.upsert(ItemPatch {
            id,
            status: Some(ToolStatus::Error),
            ..ItemPatch::default()
        });
    }
    host.with_thread(|thread, _| thread.settle_all_approvals(false, |_| true));
    let actions = sink.actions();
    if let (Some(history), Some(text)) = (&history, &text)
        && !actions.is_empty()
    {
        let turn = AgentTurn {
            provider: turn_provider,
            prompt: text.clone(),
            actions,
            summary: sink.reply(),
            thread: Some(host.thread_id().to_string()),
        };
        save_history(host.as_ref(), &sink, history, Some(turn)).await;
    }
    host.with_thread(|thread, _| {
        thread.turn = None;
        thread.updated_at = js::now_iso();
        thread.sidebar.turn_ended_at = Some(thread.updated_at.clone());
    });
    host.release();
    host.changed();
    host.with_thread(|thread, out| thread.save(out));
    host.refresh_resolve();
}
