// The desktop app's updates as this server sees them. The app (the GPUI window, before it Electron's desktop/main.ts)
// tells the server its update state; the server shows it to every page, this computer's and the user's other
// computers', and asks the app to restart into a new release once nothing is busy with Resolve. A restart in the
// middle of a turn would stop the agent halfway through an edit.
//
// AppUpdates keeps the TS's messages (DesktopToServer in, ServerToDesktop out). The Rust app runs the server in its
// own process, so with_updater connects them straight to a DesktopUpdater (Velopack, in crates/app) instead of IPC.

use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use linker_edit_protocol::{
    AppUpdate, DesktopToServer, DesktopUpdateState, DesktopUpdateStatus, ServerToDesktop,
};
use tokio::task::JoinHandle;

use crate::net::{Listener, Listeners};

/// Called with the updater's new state.
pub type OnUpdateState = Arc<dyn Fn(DesktopUpdateState) + Send + Sync>;

/// The installed app's own updater, which the window implements (with Velopack). It moves its DesktopUpdateState
/// as desktop/update-state.ts did and tells on_status every change.
#[async_trait]
pub trait DesktopUpdater: Send + Sync {
    /// Where it is now.
    fn status(&self) -> DesktopUpdateState;
    /// Called with every new state.
    fn on_status(&self, listener: OnUpdateState);
    /// Looks for a newer release; where the app can install by itself, downloads it too.
    async fn check(&self);
    /// Downloads the available release.
    async fn download(&self);
    /// Restarts into the downloaded release. The caller (AppUpdates) makes sure nothing is busy with Resolve.
    /// False when there's nothing to install or the installer didn't start (the state then says why).
    async fn install(&self) -> bool;
    /// Stops a download.
    fn cancel(&self);
}

pub type SendToDesktop = Arc<dyn Fn(ServerToDesktop) + Send + Sync>;
pub type Busy = Arc<dyn Fn() -> bool + Send + Sync>;

pub struct AppUpdatesOptions {
    /// To the desktop app.
    pub send: SendToDesktop,
    /// Whether a turn, render or open is using Resolve.
    pub busy: Busy,
    /// How often to look whether Resolve is free, while waiting. 2 seconds by default.
    pub poll: Duration,
}

impl AppUpdatesOptions {
    pub fn new(send: SendToDesktop, busy: Busy) -> Self {
        AppUpdatesOptions {
            send,
            busy,
            poll: Duration::from_millis(2000),
        }
    }
}

struct State {
    state: Option<DesktopUpdateState>,
    waiting: Option<JoinHandle<()>>,
}

pub struct AppUpdates {
    me: Weak<AppUpdates>,
    send: SendToDesktop,
    busy: Busy,
    poll: Duration,
    state: Mutex<State>,
    listeners: Listeners,
}

impl AppUpdates {
    pub fn new(options: AppUpdatesOptions) -> Arc<AppUpdates> {
        Arc::new_cyclic(|me| AppUpdates {
            me: me.clone(),
            send: options.send,
            busy: options.busy,
            poll: options.poll,
            state: Mutex::new(State {
                state: None,
                waiting: None,
            }),
            listeners: Listeners::default(),
        })
    }

    /// AppUpdates for the app's own updater, in the same process: what the TS sent over IPC goes to the updater.
    pub fn with_updater(updater: Arc<dyn DesktopUpdater>, busy: Busy) -> Arc<AppUpdates> {
        let to_updater = updater.clone();
        let send: SendToDesktop = Arc::new(move |message| {
            let updater = to_updater.clone();
            match message {
                ServerToDesktop::CheckUpdate => {
                    tokio::spawn(async move { updater.check().await });
                }
                ServerToDesktop::InstallUpdate => {
                    tokio::spawn(async move {
                        updater.install().await;
                    });
                }
                ServerToDesktop::Ready { .. } => {}
            }
        });
        let updates = AppUpdates::new(AppUpdatesOptions::new(send, busy));
        let me = Arc::downgrade(&updates);
        updater.on_status(Arc::new(move |state| {
            if let Some(updates) = me.upgrade() {
                updates.receive(DesktopToServer::UpdateState {
                    state: Box::new(state),
                });
            }
        }));
        updates.receive(DesktopToServer::UpdateState {
            state: Box::new(updater.status()),
        });
        updates
    }

    /// AppMeta.appUpdate; None until the desktop app says where it is.
    pub fn status(&self) -> Option<AppUpdate> {
        let state = self.state.lock().unwrap();
        state.state.clone().map(|update| AppUpdate {
            state: update,
            waiting: state.waiting.is_some(),
        })
    }

    pub fn on_status(&self, listener: Listener) {
        self.listeners.add(listener);
    }

    fn stop_waiting(state: &mut State) {
        if let Some(waiting) = state.waiting.take() {
            waiting.abort();
        }
    }

    fn install_now(&self) {
        Self::stop_waiting(&mut self.state.lock().unwrap());
        (self.send)(ServerToDesktop::InstallUpdate);
        self.listeners.notify();
    }

    /// From the desktop app.
    pub fn receive(&self, message: DesktopToServer) {
        match message {
            DesktopToServer::InstallRequested => self.install(),
            DesktopToServer::UpdateState { state: update } => {
                {
                    let mut state = self.state.lock().unwrap();
                    if update.status != DesktopUpdateStatus::Downloaded {
                        Self::stop_waiting(&mut state);
                    }
                    state.state = Some(*update);
                }
                self.listeners.notify();
            }
        }
    }

    pub fn check(&self) {
        (self.send)(ServerToDesktop::CheckUpdate);
    }

    /// Restarts to update once nothing is busy with Resolve. Must be called inside the tokio runtime.
    pub fn install(&self) {
        let downloaded = self
            .state
            .lock()
            .unwrap()
            .state
            .as_ref()
            .is_some_and(|update| update.status == DesktopUpdateStatus::Downloaded);
        if !downloaded {
            return;
        }
        // Not under this lock: busy takes the app's, and the app takes this one to show the update.
        if (self.busy)() {
            let mut state = self.state.lock().unwrap();
            if state.waiting.is_some() {
                return;
            }
            let me = self.me.clone();
            let poll = self.poll;
            state.waiting = Some(tokio::spawn(async move {
                loop {
                    tokio::time::sleep(poll).await;
                    let Some(updates) = me.upgrade() else { return };
                    if !(updates.busy)() {
                        // Done waiting: this task ends here rather than being stopped.
                        updates.state.lock().unwrap().waiting = None;
                        updates.install_now();
                        return;
                    }
                }
            }));
            drop(state);
            self.listeners.notify();
            return;
        }
        self.install_now();
    }

    pub fn cancel(&self) {
        {
            let mut state = self.state.lock().unwrap();
            if state.waiting.is_none() {
                return;
            }
            Self::stop_waiting(&mut state);
        }
        self.listeners.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn state(status: DesktopUpdateStatus) -> DesktopUpdateState {
        let downloaded = status == DesktopUpdateStatus::Downloaded;
        DesktopUpdateState {
            status,
            current_version: "2026.1111.2".into(),
            available_version: downloaded.then(|| "2026.1112.0".into()),
            downloaded_version: downloaded.then(|| "2026.1112.0".into()),
            percent: None,
            notes: vec![],
            checked_at: None,
            error: None,
            download_url: None,
            not_applied: None,
        }
    }

    fn update(status: DesktopUpdateStatus) -> DesktopToServer {
        DesktopToServer::UpdateState {
            state: Box::new(state(status)),
        }
    }

    struct Setup {
        updates: Arc<AppUpdates>,
        sent: Arc<Mutex<Vec<ServerToDesktop>>>,
        changes: Arc<AtomicUsize>,
        busy: Arc<AtomicBool>,
    }

    impl Setup {
        fn sent(&self) -> Vec<ServerToDesktop> {
            self.sent.lock().unwrap().clone()
        }
        fn set_busy(&self, busy: bool) {
            self.busy.store(busy, Ordering::SeqCst);
        }
        fn waiting(&self) -> bool {
            self.updates.status().unwrap().waiting
        }
    }

    fn setup() -> Setup {
        let sent: Arc<Mutex<Vec<ServerToDesktop>>> = Arc::default();
        let busy = Arc::new(AtomicBool::new(false));
        let changes = Arc::new(AtomicUsize::new(0));
        let record = sent.clone();
        let is_busy = busy.clone();
        let updates = AppUpdates::new(AppUpdatesOptions {
            send: Arc::new(move |message| record.lock().unwrap().push(message)),
            busy: Arc::new(move || is_busy.load(Ordering::SeqCst)),
            poll: Duration::from_millis(1000),
        });
        let count = changes.clone();
        updates.on_status(Arc::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
        }));
        Setup {
            updates,
            sent,
            changes,
            busy,
        }
    }

    /// vi.advanceTimersByTimeAsync: runs the timers due within ms.
    async fn advance(ms: u64) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        tokio::task::yield_now().await;
    }

    #[tokio::test(start_paused = true)]
    async fn has_nothing_to_show_until_the_desktop_app_says_where_it_is() {
        let setup = setup();
        assert_eq!(setup.updates.status(), None);
        setup.updates.receive(update(DesktopUpdateStatus::UpToDate));
        let status = setup.updates.status().unwrap();
        assert_eq!(
            (status.state.status, status.waiting),
            (DesktopUpdateStatus::UpToDate, false)
        );
        assert_eq!(setup.changes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn asks_the_app_to_check() {
        let setup = setup();
        setup.updates.check();
        assert_eq!(setup.sent(), [ServerToDesktop::CheckUpdate]);
    }

    #[tokio::test(start_paused = true)]
    async fn installs_right_away_when_resolve_is_free() {
        let setup = setup();
        setup
            .updates
            .receive(update(DesktopUpdateStatus::Downloaded));
        setup.updates.install();
        assert_eq!(setup.sent(), [ServerToDesktop::InstallUpdate]);
    }

    #[tokio::test(start_paused = true)]
    async fn does_nothing_without_a_downloaded_update() {
        let setup = setup();
        setup.updates.install();
        setup
            .updates
            .receive(update(DesktopUpdateStatus::Downloading));
        setup.updates.install();
        assert_eq!(setup.sent(), []);
    }

    #[tokio::test(start_paused = true)]
    async fn waits_for_a_running_turn_to_end() {
        let setup = setup();
        setup
            .updates
            .receive(update(DesktopUpdateStatus::Downloaded));
        setup.set_busy(true);
        setup.updates.install();
        assert!(setup.waiting());
        advance(3000).await;
        assert_eq!(setup.sent(), []);
        setup.set_busy(false);
        advance(1000).await;
        assert_eq!(setup.sent(), [ServerToDesktop::InstallUpdate]);
        assert!(!setup.waiting());
    }

    #[tokio::test(start_paused = true)]
    async fn can_be_told_not_to_restart_after_all() {
        let setup = setup();
        setup
            .updates
            .receive(update(DesktopUpdateStatus::Downloaded));
        setup.set_busy(true);
        setup.updates.install();
        setup.updates.cancel();
        setup.set_busy(false);
        advance(5000).await;
        assert_eq!(setup.sent(), []);
        assert!(!setup.waiting());
    }

    #[tokio::test(start_paused = true)]
    async fn waits_the_same_way_when_a_page_of_an_older_linker_edit_asks_through_the_app() {
        let setup = setup();
        setup
            .updates
            .receive(update(DesktopUpdateStatus::Downloaded));
        setup.set_busy(true);
        setup.updates.receive(DesktopToServer::InstallRequested);
        assert!(setup.waiting());
        setup.set_busy(false);
        advance(1000).await;
        assert_eq!(setup.sent(), [ServerToDesktop::InstallUpdate]);
    }

    #[tokio::test(start_paused = true)]
    async fn stops_waiting_when_a_newer_release_replaces_the_downloaded_one() {
        let setup = setup();
        setup
            .updates
            .receive(update(DesktopUpdateStatus::Downloaded));
        setup.set_busy(true);
        setup.updates.install();
        setup
            .updates
            .receive(update(DesktopUpdateStatus::Downloading));
        setup.set_busy(false);
        advance(5000).await;
        assert_eq!(setup.sent(), []);
    }

    /// An updater that records what it was asked.
    struct FakeUpdater {
        state: Mutex<DesktopUpdateState>,
        listeners: Mutex<Vec<OnUpdateState>>,
        asked: Mutex<Vec<&'static str>>,
        done: tokio::sync::Notify,
    }

    #[async_trait]
    impl DesktopUpdater for FakeUpdater {
        fn status(&self) -> DesktopUpdateState {
            self.state.lock().unwrap().clone()
        }
        fn on_status(&self, listener: OnUpdateState) {
            self.listeners.lock().unwrap().push(listener);
        }
        async fn check(&self) {
            self.asked.lock().unwrap().push("check");
            let next = state(DesktopUpdateStatus::Downloaded);
            *self.state.lock().unwrap() = next.clone();
            let listeners = self.listeners.lock().unwrap().clone();
            for listener in listeners {
                listener(next.clone());
            }
            self.done.notify_one();
        }
        async fn download(&self) {}
        async fn install(&self) -> bool {
            self.asked.lock().unwrap().push("install");
            self.done.notify_one();
            true
        }
        fn cancel(&self) {}
    }

    #[tokio::test]
    async fn drives_the_apps_own_updater_in_the_same_process() {
        let updater = Arc::new(FakeUpdater {
            state: Mutex::new(state(DesktopUpdateStatus::Idle)),
            listeners: Mutex::default(),
            asked: Mutex::default(),
            done: tokio::sync::Notify::new(),
        });
        let updates = AppUpdates::with_updater(updater.clone(), Arc::new(|| false));
        assert_eq!(
            updates.status().unwrap().state.status,
            DesktopUpdateStatus::Idle
        );
        updates.check();
        updater.done.notified().await;
        assert_eq!(
            updates.status().unwrap().state.status,
            DesktopUpdateStatus::Downloaded
        );
        updates.install();
        updater.done.notified().await;
        assert_eq!(*updater.asked.lock().unwrap(), ["check", "install"]);
    }
}
