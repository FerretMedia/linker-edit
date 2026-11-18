//! The installed app's own updates, from the GitHub releases through Velopack. It looks 15 seconds after the app
//! starts and then every hour, downloads by itself, and installs only when asked: the server asks once nothing
//! is busy with Resolve (linker_edit_server::app_update). Quitting with an update downloaded installs it too.
//!
//! Only a copy Velopack installed has an updater; from npm or a build from source, [`velopack_updater`] is None
//! and the server looks on npm instead. `LINKER_EDIT_UPDATE_FEED` points it at another feed, a folder or an
//! address, for apps/desktop/scripts/update-smoke.ts.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use linker_edit_protocol::{DesktopUpdateState, DesktopUpdateStatus};
use linker_edit_server::app_update::{DesktopUpdater, OnUpdateState};

use crate::update_state::{self, InstallRecord};

/// Where releases are.
const REPOSITORY: &str = "https://github.com/FerretMedia/linker-edit";
pub const FIRST_CHECK: Duration = Duration::from_secs(15);
pub const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);

/// A release on the feed.
#[derive(Clone)]
pub struct Release {
    pub version: String,
    /// What's new, as HTML.
    pub notes: String,
    info: Option<Box<velopack::UpdateInfo>>,
}

/// What the updater needs from Velopack; tests stand in for it. Each call blocks.
pub trait Feed: Send + Sync + 'static {
    /// The version installed.
    fn current_version(&self) -> String;
    /// A newer release, if there is one.
    fn check(&self) -> anyhow::Result<Option<Release>>;
    /// Downloads it, telling progress the percent done.
    fn download(
        &self,
        release: &Release,
        progress: &(dyn Fn(f64) + Send + Sync),
    ) -> anyhow::Result<()>;
    /// Starts Velopack's updater, which installs the downloaded release once this process has quit, then opens
    /// the app again if `restart`.
    fn apply(&self, release: &Release, restart: bool) -> anyhow::Result<()>;
}

/// The updater for this copy of the app, or None when Velopack didn't install it.
pub fn velopack_updater(data_dir: PathBuf) -> Option<Arc<Updater>> {
    let feed = VelopackFeed::new()?;
    Some(Updater::new(
        Arc::new(feed),
        data_dir.join("update-install.json"),
        Arc::new(|| std::process::exit(0)),
    ))
}

struct VelopackFeed(velopack::UpdateManager);

impl VelopackFeed {
    fn new() -> Option<VelopackFeed> {
        use velopack::sources::{FileSource, GithubSource, HttpSource, UpdateSource};
        let source: Box<dyn UpdateSource> = match std::env::var("LINKER_EDIT_UPDATE_FEED") {
            Ok(feed) if feed.starts_with("http://") || feed.starts_with("https://") => {
                Box::new(HttpSource::new(feed))
            }
            Ok(feed) if !feed.is_empty() => Box::new(FileSource::new(feed)),
            _ => Box::new(GithubSource::new(REPOSITORY, None, false)),
        };
        // Fails when this copy wasn't installed by Velopack.
        velopack::UpdateManager::new_boxed(source, None, None)
            .ok()
            .map(VelopackFeed)
    }
}

impl Feed for VelopackFeed {
    fn current_version(&self) -> String {
        self.0.get_current_version_as_string()
    }

    fn check(&self) -> anyhow::Result<Option<Release>> {
        Ok(match self.0.check_for_updates()? {
            velopack::UpdateCheck::UpdateAvailable(info) => {
                let asset = &info.TargetFullRelease;
                Some(Release {
                    version: asset.Version.clone(),
                    notes: if asset.NotesHtml.is_empty() {
                        asset.NotesMarkdown.clone()
                    } else {
                        asset.NotesHtml.clone()
                    },
                    info: Some(info),
                })
            }
            velopack::UpdateCheck::NoUpdateAvailable | velopack::UpdateCheck::RemoteIsEmpty => None,
        })
    }

    fn download(
        &self,
        release: &Release,
        progress: &(dyn Fn(f64) + Send + Sync),
    ) -> anyhow::Result<()> {
        let info = release.info.as_ref().expect("a release from check");
        let (sender, receiver) = std::sync::mpsc::channel::<i16>();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for percent in receiver {
                    progress(f64::from(percent));
                }
            });
            // The sender goes with the download, which ends the thread above.
            self.0.download_updates(info, Some(sender))
        })?;
        Ok(())
    }

    fn apply(&self, release: &Release, restart: bool) -> anyhow::Result<()> {
        let info = release.info.as_ref().expect("a release from check");
        self.0.wait_exit_then_apply_updates(
            &info.TargetFullRelease,
            true,
            restart,
            Vec::<String>::new(),
        )?;
        Ok(())
    }
}

#[derive(Default)]
struct Inner {
    state: Option<DesktopUpdateState>,
    available: Option<Release>,
    downloaded: Option<Release>,
    checking: bool,
    downloading: bool,
    installing: bool,
}

pub struct Updater {
    me: Weak<Updater>,
    feed: Arc<dyn Feed>,
    record_file: PathBuf,
    /// Quits the app once Velopack's updater waits for it: std::process::exit, which leaves the agents and ffmpeg
    /// running; on Windows the updater stops what still runs from the app's folder before replacing it.
    exit: Arc<dyn Fn() + Send + Sync>,
    current: String,
    inner: Mutex<Inner>,
    listeners: Mutex<Vec<OnUpdateState>>,
}

fn now() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

impl Updater {
    pub fn new(
        feed: Arc<dyn Feed>,
        record_file: PathBuf,
        exit: Arc<dyn Fn() + Send + Sync>,
    ) -> Arc<Updater> {
        let current = feed.current_version();
        let record = std::fs::read_to_string(&record_file)
            .ok()
            .and_then(|text| serde_json::from_str::<InstallRecord>(&text).ok());
        let not_applied = update_state::not_applied_version(record.as_ref(), &current);
        match &not_applied {
            Some(version) => tracing::warn!(
                target: "updates",
                "The install of {version} didn't apply: still on {current}."
            ),
            None => {
                let _ = std::fs::remove_file(&record_file);
            }
        }
        Arc::new_cyclic(|me| Updater {
            me: me.clone(),
            feed,
            record_file,
            exit,
            inner: Mutex::new(Inner {
                state: Some(update_state::initial(&current, not_applied)),
                ..Inner::default()
            }),
            current,
            listeners: Mutex::default(),
        })
    }

    /// Checks 15 seconds from now, then every hour. Must be called inside the tokio runtime.
    pub fn start(&self) {
        let me = self.me.clone();
        tokio::spawn(async move {
            tokio::time::sleep(FIRST_CHECK).await;
            loop {
                let Some(updater) = me.upgrade() else { return };
                updater.check().await;
                drop(updater);
                tokio::time::sleep(CHECK_EVERY).await;
            }
        });
    }

    fn state(&self) -> DesktopUpdateState {
        self.inner.lock().unwrap().state.clone().expect("a state")
    }

    fn set(&self, change: impl FnOnce(&DesktopUpdateState) -> DesktopUpdateState) {
        let next = {
            let mut inner = self.inner.lock().unwrap();
            let state = inner.state.as_ref().expect("a state");
            let next = change(state);
            if next.status != state.status || next.error != state.error {
                tracing::info!(
                    target: "updates",
                    "{:?}{}{}",
                    next.status,
                    next.available_version.as_ref().map(|version| format!(" {version}")).unwrap_or_default(),
                    next.error.as_ref().map(|error| format!(": {error}")).unwrap_or_default()
                );
            }
            inner.state = Some(next.clone());
            next
        };
        let listeners = self.listeners.lock().unwrap().clone();
        for listener in listeners {
            listener(next.clone());
        }
    }

    fn write_record(&self, record: Option<InstallRecord>) {
        let result = match record {
            Some(record) => std::fs::write(
                &self.record_file,
                serde_json::to_string(&record).expect("a record serializes"),
            ),
            None => match std::fs::remove_file(&self.record_file) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                other => other,
            },
        };
        if let Err(error) = result {
            tracing::error!(target: "updates", "Couldn't keep the update record: {error}");
        }
    }

    /// Before the app quits: Velopack installs a downloaded release once it has.
    pub fn quitting(&self) {
        let release = {
            let inner = self.inner.lock().unwrap();
            let downloaded = inner.state.as_ref().map(|state| state.status)
                == Some(DesktopUpdateStatus::Downloaded);
            if inner.installing || !downloaded {
                return;
            }
            inner.downloaded.clone()
        };
        let Some(release) = release else { return };
        self.write_record(Some(InstallRecord {
            from: self.current.clone(),
            to: release.version.clone(),
        }));
        if let Err(error) = self.feed.apply(&release, false) {
            tracing::error!(target: "updates", "Couldn't install {} on quit: {error:#}", release.version);
            self.write_record(None);
        }
    }
}

/// Runs a Feed call off the async threads; a panic in it is an error.
async fn blocking<T: Send + 'static>(
    call: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> anyhow::Result<T> {
    tokio::task::spawn_blocking(call)
        .await
        .unwrap_or_else(|error| Err(anyhow::anyhow!("{error}")))
}

#[async_trait]
impl DesktopUpdater for Updater {
    fn status(&self) -> DesktopUpdateState {
        self.state()
    }

    fn on_status(&self, listener: OnUpdateState) {
        self.listeners.lock().unwrap().push(listener);
    }

    async fn check(&self) {
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.checking || inner.installing || inner.downloading {
                return;
            }
            inner.checking = true;
        }
        self.set(update_state::on_check_start);
        let feed = self.feed.clone();
        let found = blocking(move || feed.check()).await;
        self.inner.lock().unwrap().checking = false;
        match found {
            Ok(Some(release)) => {
                let notes = update_state::release_note_lines(&release.notes);
                self.set(|state| {
                    update_state::on_available(state, &release.version, notes, &now(), None)
                });
                self.inner.lock().unwrap().available = Some(release);
                self.download().await;
            }
            Ok(None) => self.set(|state| update_state::on_not_available(state, &now())),
            Err(error) => {
                let message = format!("{error:#}");
                self.set(|state| update_state::on_check_failed(state, &message, &now()));
            }
        }
    }

    async fn download(&self) {
        let release = {
            let mut inner = self.inner.lock().unwrap();
            let Some(release) = inner.available.clone() else {
                return;
            };
            let have = inner.downloaded.as_ref().map(|release| &release.version);
            if inner.downloading || inner.installing || have == Some(&release.version) {
                return;
            }
            inner.downloading = true;
            release
        };
        self.set(|state| update_state::on_progress(state, 0.0));
        let (feed, me, downloading) = (self.feed.clone(), self.me.clone(), release.clone());
        let result = blocking(move || {
            feed.download(&downloading, &move |percent| {
                if let Some(updater) = me.upgrade() {
                    let shown = updater.state().percent;
                    if shown != Some(percent.clamp(0.0, 100.0).floor()) {
                        updater.set(|state| update_state::on_progress(state, percent));
                    }
                }
            })
        })
        .await;
        self.inner.lock().unwrap().downloading = false;
        match result {
            Ok(()) => {
                self.set(|state| update_state::on_downloaded(state, &release.version));
                self.inner.lock().unwrap().downloaded = Some(release);
            }
            Err(error) => {
                let message = format!("{error:#}");
                self.set(|state| update_state::on_download_failed(state, &message));
            }
        }
    }

    /// Restarts into the downloaded release: starts Velopack's updater, which waits for this process to quit,
    /// installs, and opens the app again. False when there's nothing to install or the updater didn't start.
    async fn install(&self) -> bool {
        let release = {
            let mut inner = self.inner.lock().unwrap();
            let downloaded = inner.state.as_ref().map(|state| state.status)
                == Some(DesktopUpdateStatus::Downloaded);
            let Some(release) = inner
                .downloaded
                .clone()
                .filter(|_| downloaded && !inner.installing)
            else {
                return false;
            };
            inner.installing = true;
            release
        };
        tracing::info!(target: "updates", "Installing {}", release.version);
        self.write_record(Some(InstallRecord {
            from: self.current.clone(),
            to: release.version.clone(),
        }));
        let (feed, applying) = (self.feed.clone(), release.clone());
        match blocking(move || feed.apply(&applying, true)).await {
            Ok(()) => {
                (self.exit)();
                true
            }
            Err(error) => {
                self.inner.lock().unwrap().installing = false;
                self.write_record(None);
                let message = format!("Couldn't start the installer: {error:#}");
                self.set(|state| update_state::on_install_failed(state, &message));
                false
            }
        }
    }

    /// Velopack can't stop a download halfway; the server never asks to.
    fn cancel(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Stands in for Velopack: offers `offer`, and records what it was asked.
    #[derive(Default)]
    struct FakeFeed {
        offer: Mutex<Option<String>>,
        fail_check: Mutex<Option<String>>,
        fail_download: Mutex<Option<String>>,
        fail_apply: Mutex<Option<String>>,
        asked: Mutex<Vec<String>>,
    }

    impl Feed for FakeFeed {
        fn current_version(&self) -> String {
            "2026.1111.2".into()
        }
        fn check(&self) -> anyhow::Result<Option<Release>> {
            self.asked.lock().unwrap().push("check".into());
            if let Some(error) = self.fail_check.lock().unwrap().clone() {
                anyhow::bail!(error);
            }
            Ok(self.offer.lock().unwrap().clone().map(|version| Release {
                version,
                notes: "<ul><li>Faster previews by @a in #9</li></ul>".into(),
                info: None,
            }))
        }
        fn download(
            &self,
            release: &Release,
            progress: &(dyn Fn(f64) + Send + Sync),
        ) -> anyhow::Result<()> {
            self.asked
                .lock()
                .unwrap()
                .push(format!("download {}", release.version));
            progress(12.5);
            progress(12.9);
            match self.fail_download.lock().unwrap().clone() {
                Some(error) => anyhow::bail!(error),
                None => Ok(()),
            }
        }
        fn apply(&self, release: &Release, restart: bool) -> anyhow::Result<()> {
            self.asked
                .lock()
                .unwrap()
                .push(format!("apply {} restart={restart}", release.version));
            match self.fail_apply.lock().unwrap().clone() {
                Some(error) => anyhow::bail!(error),
                None => Ok(()),
            }
        }
    }

    struct Setup {
        feed: Arc<FakeFeed>,
        updater: Arc<Updater>,
        states: Arc<Mutex<Vec<DesktopUpdateState>>>,
        exited: Arc<AtomicBool>,
        record: PathBuf,
        _dir: tempfile::TempDir,
    }

    fn setup(record: Option<InstallRecord>) -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("update-install.json");
        if let Some(record) = record {
            std::fs::write(&file, serde_json::to_string(&record).unwrap()).unwrap();
        }
        let feed = Arc::new(FakeFeed::default());
        let exited = Arc::new(AtomicBool::new(false));
        let exit = exited.clone();
        let updater = Updater::new(
            feed.clone(),
            file.clone(),
            Arc::new(move || exit.store(true, Ordering::SeqCst)),
        );
        let states: Arc<Mutex<Vec<DesktopUpdateState>>> = Arc::default();
        let seen = states.clone();
        updater.on_status(Arc::new(move |state| seen.lock().unwrap().push(state)));
        Setup {
            feed,
            updater,
            states,
            exited,
            record: file,
            _dir: dir,
        }
    }

    fn record(setup: &Setup) -> Option<InstallRecord> {
        std::fs::read_to_string(&setup.record)
            .ok()
            .map(|text| serde_json::from_str(&text).unwrap())
    }

    fn asked(setup: &Setup) -> Vec<String> {
        setup.feed.asked.lock().unwrap().clone()
    }

    async fn downloaded(setup: &Setup) {
        *setup.feed.offer.lock().unwrap() = Some("2026.1112.0".into());
        setup.updater.check().await;
    }

    #[tokio::test]
    async fn follows_the_feed_to_a_downloaded_update_with_its_notes() {
        let setup = setup(None);
        downloaded(&setup).await;
        let state = setup.updater.status();
        assert_eq!(state.status, DesktopUpdateStatus::Downloaded);
        assert_eq!(state.downloaded_version.as_deref(), Some("2026.1112.0"));
        assert_eq!(state.notes, ["Faster previews"]);
        let statuses: Vec<_> = setup
            .states
            .lock()
            .unwrap()
            .iter()
            .map(|state| (state.status, state.percent))
            .collect();
        // 12.9 shows as 12 too, so it isn't sent again.
        assert_eq!(
            statuses,
            [
                (DesktopUpdateStatus::Checking, None),
                (DesktopUpdateStatus::Available, None),
                (DesktopUpdateStatus::Downloading, Some(0.0)),
                (DesktopUpdateStatus::Downloading, Some(12.0)),
                (DesktopUpdateStatus::Downloaded, Some(100.0)),
            ]
        );
        assert_eq!(asked(&setup), ["check", "download 2026.1112.0"]);
        // A later check finds the same release and doesn't download it again.
        setup.updater.check().await;
        assert_eq!(asked(&setup), ["check", "download 2026.1112.0", "check"]);
        assert_eq!(
            setup.updater.status().status,
            DesktopUpdateStatus::Downloaded
        );
    }

    #[tokio::test]
    async fn says_up_to_date() {
        let setup = setup(None);
        setup.updater.check().await;
        assert_eq!(setup.updater.status().status, DesktopUpdateStatus::UpToDate);
    }

    #[tokio::test]
    async fn tells_a_failed_download_from_a_failed_check() {
        let setup = setup(None);
        *setup.feed.fail_check.lock().unwrap() = Some("offline".into());
        setup.updater.check().await;
        let state = setup.updater.status();
        assert_eq!(
            (state.status, state.error.as_deref()),
            (DesktopUpdateStatus::Error, Some("offline"))
        );
        *setup.feed.fail_check.lock().unwrap() = None;
        *setup.feed.fail_download.lock().unwrap() = Some("connection reset".into());
        downloaded(&setup).await;
        let state = setup.updater.status();
        assert_eq!(
            (state.status, state.error.as_deref()),
            (DesktopUpdateStatus::Available, Some("connection reset"))
        );
    }

    #[tokio::test]
    async fn keeps_a_record_then_installs_silently_and_quits() {
        let setup = setup(None);
        downloaded(&setup).await;
        assert!(setup.updater.install().await);
        assert!(setup.exited.load(Ordering::SeqCst));
        assert_eq!(
            record(&setup),
            Some(InstallRecord {
                from: "2026.1111.2".into(),
                to: "2026.1112.0".into()
            })
        );
        assert_eq!(
            asked(&setup).last().unwrap(),
            "apply 2026.1112.0 restart=true"
        );
        // Quitting for the install doesn't install again.
        setup.updater.quitting();
        assert_eq!(asked(&setup).len(), 3);
    }

    #[tokio::test]
    async fn has_nothing_to_install_before_a_download() {
        let setup = setup(None);
        assert!(!setup.updater.install().await);
        assert!(asked(&setup).is_empty());
        assert!(!setup.exited.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn keeps_running_when_the_updater_doesnt_start() {
        let setup = setup(None);
        downloaded(&setup).await;
        *setup.feed.fail_apply.lock().unwrap() = Some("Update.exe is missing".into());
        assert!(!setup.updater.install().await);
        assert!(!setup.exited.load(Ordering::SeqCst));
        assert_eq!(record(&setup), None);
        let state = setup.updater.status();
        assert_eq!(state.status, DesktopUpdateStatus::Downloaded);
        assert_eq!(
            state.error.as_deref(),
            Some("Couldn't start the installer: Update.exe is missing")
        );
    }

    #[tokio::test]
    async fn installs_on_quit_with_a_record() {
        let setup = setup(None);
        setup.updater.quitting();
        assert!(asked(&setup).is_empty());
        downloaded(&setup).await;
        setup.updater.quitting();
        assert_eq!(
            asked(&setup).last().unwrap(),
            "apply 2026.1112.0 restart=false"
        );
        assert_eq!(record(&setup).unwrap().to, "2026.1112.0");
    }

    #[test]
    fn says_when_the_last_install_didnt_apply_and_forgets_a_record_that_did() {
        let failed = setup(Some(InstallRecord {
            from: "2026.1111.2".into(),
            to: "2026.1112.0".into(),
        }));
        assert_eq!(
            failed.updater.status().not_applied.as_deref(),
            Some("2026.1112.0")
        );
        assert!(record(&failed).is_some());
        let worked = setup(Some(InstallRecord {
            from: "2026.1111.1".into(),
            to: "2026.1111.2".into(),
        }));
        assert_eq!(worked.updater.status().not_applied, None);
        assert_eq!(record(&worked), None);
    }
}
