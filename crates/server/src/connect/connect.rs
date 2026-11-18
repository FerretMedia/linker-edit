// Linker Connect, copied from T3 Connect: your Linker account links up to 3 edit computers. Each gets an address at
// Linker (https://edit-….linker.cool) that a Cloudflare Tunnel brings to this server on 127.0.0.1, so your other
// computers reach it from anywhere, without Tailscale. Linker only makes and removes the tunnels
// (app.linker.cool/api/edit/computers); the traffic goes straight through the tunnel to this computer.
//
// Getting in (the http layer): your other Linker-Edit, or the iPad app, shows this computer your Linker token, this
// computer asks Linker whose it is, and only the account signed in here gets a one-time code for a session
// (sessions.rs): a cookie for Linker-Edit's window, a Bearer token for the iPad app (redeem).

use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use anyhow::anyhow;
use linker_edit_protocol::{
    ConnectDeviceKind, LinkedComputer, LinkerAccount, LinkerConnect, LinkerConnectState,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::task::{AbortHandle, JoinHandle};
use url::Url;

use super::cloudflared::{
    CloudflaredTunnels, OnPercent, PrepareCloudflared, StartTunnel, Tunnel, TunnelEvent,
};
use super::sessions::Sessions;
use crate::linker::LinkerService;
use crate::net::{
    Clock, Fetch, HttpFetch, Listener, Listeners, Request, encode_uri_component, iso_date,
    pretty_json, slice_utf16, system_clock, write_private,
};

pub const LIMIT: u32 = 3;
const LIST_FRESH_FOR: i64 = 30_000;
const NETWORK_TIMEOUT: Duration = Duration::from_secs(20);
/// Waits before starting a stopped tunnel again, growing up to the last.
const RESTART_DELAYS: [u64; 5] = [2_000, 5_000, 15_000, 30_000, 60_000];
/// Sign-in attempts through the tunnel per minute, each way in, so nobody makes this computer hammer Linker.
const ATTEMPTS_PER_MINUTE: usize = 20;

/// True while fewer than ATTEMPTS_PER_MINUTE tries came in the last minute, counting this one.
#[derive(Default)]
struct RateLimit(Vec<i64>);

impl RateLimit {
    fn allow(&mut self, now: i64) -> bool {
        self.0.retain(|at| now - at < 60_000);
        if self.0.len() >= ATTEMPTS_PER_MINUTE {
            return false;
        }
        self.0.push(now);
        true
    }
}

/// ~/LinkerEdit/connect.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    computer_id: String,
    enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tunnel_token: Option<String>,
    /// The Linker account that linked this computer, the only one let in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner_id: Option<String>,
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn read(file: &PathBuf) -> Saved {
    let value = std::fs::read_to_string(file)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    let text =
        |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
    match value {
        // First run, or a damaged file: a new id, linked again when turned on.
        Some(value) if text(&value, "computerId").is_some_and(|id| !id.is_empty()) => Saved {
            computer_id: text(&value, "computerId").unwrap_or_default(),
            enabled: value.get("enabled") == Some(&Value::Bool(true)),
            url: text(&value, "url"),
            tunnel_token: text(&value, "tunnelToken"),
            owner_id: text(&value, "ownerId"),
        },
        _ => Saved {
            computer_id: new_id(),
            enabled: false,
            url: None,
            tunnel_token: None,
            owner_id: None,
        },
    }
}

pub struct ConnectOptions {
    /// ~/LinkerEdit/connect.json, readable only by this user: it holds the tunnel's token.
    pub file: PathBuf,
    /// This server's port, which the tunnel forwards to.
    pub port: u16,
    pub linker: Arc<dyn LinkerService>,
    /// Linker's dashboard and API (app.linker.cool), where computers are linked.
    pub app_url: String,
    pub sessions: Arc<Sessions>,
    /// Finds or downloads cloudflared (cloudflared::Cloudflared).
    pub prepare: Arc<dyn PrepareCloudflared>,
    pub start_tunnel: Arc<dyn StartTunnel>,
    pub fetch: Arc<dyn Fetch>,
    /// This computer's name, os.hostname().
    pub name: String,
    pub now: Clock,
}

impl ConnectOptions {
    /// With the real cloudflared, network, name and clock.
    pub fn new(
        file: PathBuf,
        port: u16,
        linker: Arc<dyn LinkerService>,
        app_url: String,
        sessions: Arc<Sessions>,
        prepare: Arc<dyn PrepareCloudflared>,
    ) -> Self {
        ConnectOptions {
            file,
            port,
            linker,
            app_url,
            sessions,
            prepare,
            start_tunnel: Arc::new(CloudflaredTunnels),
            fetch: Arc::new(HttpFetch::default()),
            name: gethostname::gethostname().to_string_lossy().into_owned(),
            now: system_clock(),
        }
    }
}

/// What Linker answers, worded for the user.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
struct RelayError {
    status: u16,
    body: Map<String, Value>,
    message: String,
}

fn relay_message(status: u16, body: &Map<String, Value>) -> String {
    if status == 401 {
        return "Linker didn't accept your sign-in. Sign out of Linker and in again.".into();
    }
    if status == 404 || status == 503 {
        return "Linker Connect isn't available yet. Try again after Linker's next release.".into();
    }
    if body.get("error").and_then(Value::as_str) == Some("cloudflare") {
        let message = match body.get("message") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(text)) => text.clone(),
            Some(other) => other.to_string(),
        };
        return format!("Linker couldn't set up the tunnel. Try again in a minute. ({message})");
    }
    if let Some(message) = body
        .get("message")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
    {
        return message.to_string();
    }
    format!("Linker answered {status}.")
}

fn is_computer(value: &Value) -> Option<LinkedComputer> {
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
    let url = text("url").filter(|url| url.starts_with("https://"))?;
    Some(LinkedComputer {
        id: text("id")?,
        name: text("name")?,
        url,
        online: value.get("online").and_then(Value::as_bool),
        linked_at: text("linkedAt").unwrap_or_default(),
        last_seen_at: text("lastSeenAt"),
    })
}

fn computers_of(value: Option<&Value>) -> Option<Vec<LinkedComputer>> {
    value
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(is_computer).collect())
}

/// A refusal at /api/connect/session or /api/connect/native: answer status with `{ "error": error }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Refusal {
    #[serde(skip)]
    pub status: u16,
    pub error: String,
}

fn refuse(status: u16, error: impl Into<String>) -> Refusal {
    Refusal {
        status,
        error: error.into(),
    }
}

/// /api/connect/native's answer: the iPad app's Bearer session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Redeemed {
    pub session: String,
    /// ISO 8601.
    pub expires_at: String,
}

struct State {
    saved: Saved,
    starting: bool,
    step: Option<String>,
    problem: Option<String>,
    connected: bool,
    tunnel: Option<Box<dyn Tunnel>>,
    /// Counts tunnels started, so a stopped one's events change nothing.
    tunnel_run: u64,
    /// A tunnel that stopped before it was kept (its program never started).
    ended_run: Option<u64>,
    restart: Option<AbortHandle>,
    restarts: usize,
    computers: Option<Vec<LinkedComputer>>,
    listed_at: i64,
    at_limit: bool,
    admitting: RateLimit,
    redeeming: RateLimit,
}

impl State {
    fn clear_restart(&mut self) {
        if let Some(restart) = self.restart.take() {
            restart.abort();
        }
    }

    fn stop_tunnel(&mut self) {
        self.clear_restart();
        if let Some(tunnel) = self.tunnel.take() {
            tunnel.stop();
        }
        self.connected = false;
    }
}

pub struct Connect {
    me: Weak<Connect>,
    file: PathBuf,
    port: u16,
    linker: Arc<dyn LinkerService>,
    app_url: String,
    sessions: Arc<Sessions>,
    prepare: Arc<dyn PrepareCloudflared>,
    start_tunnel: Arc<dyn StartTunnel>,
    fetch: Arc<dyn Fetch>,
    name: String,
    now: Clock,
    state: Mutex<State>,
    listeners: Listeners,
}

impl Connect {
    pub fn new(options: ConnectOptions) -> Arc<Connect> {
        let saved = read(&options.file);
        let connect = Arc::new_cyclic(|me| Connect {
            me: me.clone(),
            file: options.file,
            port: options.port,
            linker: options.linker,
            app_url: options.app_url.trim_end_matches('/').to_string(),
            sessions: options.sessions,
            prepare: options.prepare,
            start_tunnel: options.start_tunnel,
            fetch: options.fetch,
            name: options.name,
            now: options.now,
            state: Mutex::new(State {
                saved,
                starting: false,
                step: None,
                problem: None,
                connected: false,
                tunnel: None,
                tunnel_run: 0,
                ended_run: None,
                restart: None,
                restarts: 0,
                computers: None,
                listed_at: 0,
                at_limit: false,
                admitting: RateLimit::default(),
                redeeming: RateLimit::default(),
            }),
            listeners: Listeners::default(),
        });
        let me = connect.me.clone();
        connect.sessions.on_change(Arc::new(move || {
            if let Some(connect) = me.upgrade() {
                connect.changed();
            }
        }));
        // Signing out of Linker here ends Linker Connect: nobody can be checked against the account any more.
        let me = connect.me.clone();
        connect.linker.on_status(Arc::new(move || {
            if let Some(connect) = me.upgrade() {
                connect.on_linker_status();
            }
        }));
        connect
    }

    fn now(&self) -> i64 {
        (self.now)()
    }

    fn changed(&self) {
        self.listeners.notify();
    }

    fn signed_in(&self) -> bool {
        matches!(self.linker.status(), LinkerAccount::SignedIn { .. })
    }

    fn write(&self) -> anyhow::Result<()> {
        let saved = self.state.lock().unwrap().saved.clone();
        write_private(&self.file, &pretty_json(&saved))?;
        Ok(())
    }

    fn set_problem(&self, problem: Option<String>) {
        self.state.lock().unwrap().problem = problem;
    }

    pub fn status(&self) -> LinkerConnect {
        let signed_in = self.signed_in();
        let devices = self.sessions.list();
        let state = self.state.lock().unwrap();
        let on = state.tunnel.is_some();
        let shown = if !signed_in && !on {
            LinkerConnectState::SignedOut
        } else if state.starting {
            LinkerConnectState::Starting
        } else if on {
            LinkerConnectState::On
        } else {
            LinkerConnectState::Off
        };
        LinkerConnect {
            state: shown,
            url: state.saved.url.clone(),
            connected: shown == LinkerConnectState::On && state.connected,
            step: if state.starting {
                state.step.clone()
            } else {
                None
            },
            problem: state.problem.clone(),
            computer_id: state.saved.computer_id.clone(),
            computers: state.computers.clone(),
            limit: LIMIT,
            at_limit: state.at_limit,
            devices,
        }
    }

    pub fn on_status(&self, listener: Listener) {
        self.listeners.add(listener);
    }

    async fn relay(
        &self,
        method: &str,
        route: &str,
        body: Option<Value>,
    ) -> anyhow::Result<Map<String, Value>> {
        let Some(token) = self.linker.access_token().await? else {
            return Err(RelayError {
                status: 401,
                body: Map::new(),
                message: "Sign in with Linker first.".into(),
            }
            .into());
        };
        let mut request = Request::new(
            method,
            format!("{}/api/edit/computers{route}", self.app_url),
        )
        .header("authorization", format!("Bearer {token}"))
        .header("accept", "application/json")
        .timeout(NETWORK_TIMEOUT);
        if let Some(body) = body {
            request = request
                .header("content-type", "application/json")
                .body(body.to_string());
        }
        let res = self.fetch.fetch(request).await?;
        let json = if res.status == 204 {
            Map::new()
        } else {
            match res.json_value() {
                Some(Value::Object(map)) => map,
                _ => Map::new(),
            }
        };
        if !res.ok() {
            let message = relay_message(res.status, &json);
            return Err(RelayError {
                status: res.status,
                body: json,
                message,
            }
            .into());
        }
        Ok(json)
    }

    async fn list_computers(&self) -> anyhow::Result<Vec<LinkedComputer>> {
        let body = self.relay("GET", "", None).await?;
        let computers = computers_of(body.get("computers")).unwrap_or_default();
        let mut state = self.state.lock().unwrap();
        state.computers = Some(computers.clone());
        state.listed_at = self.now();
        Ok(computers)
    }

    fn on_tunnel(&self, run: u64, event: TunnelEvent) {
        {
            let mut state = self.state.lock().unwrap();
            if run != state.tunnel_run {
                return;
            }
            match event {
                TunnelEvent::Up => {
                    state.connected = true;
                    state.restarts = 0;
                    state.problem = None;
                }
                TunnelEvent::Connecting => state.connected = false,
                TunnelEvent::Down { detail } => {
                    // cloudflared stopped by itself: start it again while Linker Connect is on.
                    state.connected = false;
                    state.tunnel = None;
                    state.ended_run = Some(run);
                    state.problem = Some(match detail {
                        Some(detail) if !detail.is_empty() => {
                            format!("The tunnel stopped: {detail}")
                        }
                        _ => "The tunnel stopped.".into(),
                    });
                    if state.saved.enabled && state.saved.tunnel_token.is_some() {
                        let delay = RESTART_DELAYS[state.restarts.min(RESTART_DELAYS.len() - 1)];
                        state.restarts += 1;
                        state.clear_restart();
                        let me = self.me.clone();
                        let restart = tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_millis(delay)).await;
                            let Some(connect) = me.upgrade() else { return };
                            // Running now: stopping the tunnel mustn't cancel this start halfway.
                            connect.state.lock().unwrap().restart = None;
                            connect.restart_tunnel().await;
                        });
                        state.restart = Some(restart.abort_handle());
                    }
                }
            }
        }
        self.changed();
    }

    async fn restart_tunnel(&self) {
        if let Err(error) = self.run_tunnel().await {
            self.set_problem(Some(error.to_string()));
            self.changed();
        }
    }

    async fn run_tunnel(&self) -> anyhow::Result<()> {
        {
            let mut state = self.state.lock().unwrap();
            state.clear_restart();
            if state.saved.tunnel_token.is_none() || state.tunnel.is_some() {
                return Ok(());
            }
            state.step = Some("Getting cloudflared".into());
        }
        self.changed();
        let me = self.me.clone();
        let on_percent: OnPercent = Arc::new(move |percent| {
            if let Some(connect) = me.upgrade() {
                connect.state.lock().unwrap().step =
                    Some(format!("Downloading cloudflared · {percent}%"));
                connect.changed();
            }
        });
        let file = self.prepare.prepare(on_percent).await?;
        let (token, run) = {
            let mut state = self.state.lock().unwrap();
            let Some(token) = state.saved.tunnel_token.clone() else {
                return Ok(());
            };
            if !state.saved.enabled || state.tunnel.is_some() {
                return Ok(());
            }
            state.tunnel_run += 1;
            (token, state.tunnel_run)
        };
        let me = self.me.clone();
        let tunnel = self.start_tunnel.start(
            &file,
            &token,
            Arc::new(move |event| {
                if let Some(connect) = me.upgrade() {
                    connect.on_tunnel(run, event);
                }
            }),
        );
        {
            let mut state = self.state.lock().unwrap();
            if state.tunnel_run == run && state.ended_run != Some(run) && state.tunnel.is_none() {
                state.tunnel = Some(tunnel);
            } else {
                tunnel.stop();
            }
        }
        self.changed();
        Ok(())
    }

    /// Links this computer at Linker, or refreshes its link; false when it couldn't.
    async fn link(&self) -> anyhow::Result<bool> {
        if self.linker.user_id().is_none() {
            self.linker.refresh().await;
        }
        let Some(user_id) = self.linker.user_id() else {
            self.set_problem(Some(
                "Linker didn't say who you are. Sign out of Linker and in again.".into(),
            ));
            return Ok(false);
        };
        // Another account signed in here: the old link isn't theirs, nor are the devices the old one signed in.
        let other_account = {
            let mut state = self.state.lock().unwrap();
            let other = state
                .saved
                .owner_id
                .as_ref()
                .is_some_and(|owner| *owner != user_id);
            if other {
                state.saved.url = None;
                state.saved.tunnel_token = None;
                state.saved.computer_id = new_id();
            }
            other
        };
        if other_account {
            self.sessions.revoke_all();
        }
        let body = loop {
            let id = {
                let mut state = self.state.lock().unwrap();
                state.step = Some("Linking this computer".into());
                state.saved.computer_id.clone()
            };
            self.changed();
            let request = json!({ "name": slice_utf16(&self.name, 64), "port": self.port });
            match self
                .relay(
                    "PUT",
                    &format!("/{}", encode_uri_component(&id)),
                    Some(request),
                )
                .await
            {
                Ok(body) => break body,
                Err(error) => {
                    let Some(relay) = error.downcast_ref::<RelayError>() else {
                        return Err(error);
                    };
                    let kind = relay.body.get("error").and_then(Value::as_str);
                    if relay.status == 409 && kind == Some("taken") {
                        // Someone else's computer has this id, however unlikely: take a new one.
                        self.state.lock().unwrap().saved.computer_id = new_id();
                        self.write()?;
                        continue;
                    }
                    if relay.status == 409 && kind == Some("limit") {
                        let mut state = self.state.lock().unwrap();
                        state.at_limit = true;
                        if let Some(computers) = computers_of(relay.body.get("computers")) {
                            state.computers = Some(computers);
                        }
                        state.problem = Some(format!(
                            "Your Linker account already has {LIMIT} computers linked. Remove one below to link this one."
                        ));
                        return Ok(false);
                    }
                    return Err(error);
                }
            }
        };
        let computer = body.get("computer").and_then(is_computer);
        let token = body.get("tunnelToken").and_then(Value::as_str);
        let (Some(computer), Some(token)) = (computer, token) else {
            return Err(anyhow!("Linker's answer was missing the tunnel."));
        };
        {
            let mut state = self.state.lock().unwrap();
            state.saved.url = Some(computer.url);
            state.saved.tunnel_token = Some(token.to_string());
            state.saved.owner_id = Some(user_id);
            state.at_limit = false;
        }
        self.write()?;
        let _ = self.list_computers().await;
        Ok(true)
    }

    async fn turn_on(&self) {
        {
            let mut state = self.state.lock().unwrap();
            if state.starting || state.tunnel.is_some() {
                return;
            }
            state.starting = true;
            state.problem = None;
        }
        self.changed();
        let result: anyhow::Result<()> = async {
            let linked = match self.link().await {
                Ok(linked) => linked,
                Err(error) => {
                    // Offline at start: a computer linked before still opens its tunnel; Linker's side is unchanged.
                    let user_id = self.linker.user_id();
                    let mut state = self.state.lock().unwrap();
                    let offline = error.downcast_ref::<RelayError>().is_none()
                        && state.saved.tunnel_token.is_some()
                        && state.saved.owner_id.is_some()
                        && state.saved.owner_id == user_id;
                    state.problem = Some(error.to_string());
                    offline
                }
            };
            self.state.lock().unwrap().saved.enabled = linked;
            self.write()?;
            if linked {
                self.run_tunnel().await?;
            }
            Ok(())
        }
        .await;
        {
            let mut state = self.state.lock().unwrap();
            if let Err(error) = result {
                state.problem = Some(error.to_string());
                state.stop_tunnel();
            }
            state.starting = false;
            state.step = None;
        }
        self.changed();
    }

    fn turn_off(&self, reason: Option<String>) {
        {
            let mut state = self.state.lock().unwrap();
            state.stop_tunnel();
            state.saved.enabled = false;
        }
        if let Err(error) = self.write() {
            tracing::error!("Couldn't keep Linker Connect's setting: {error:#}");
        }
        // Nobody stays in through a tunnel that's off.
        self.sessions.revoke_all();
        self.set_problem(reason);
        self.changed();
    }

    fn on_linker_status(&self) {
        let signed_out = matches!(self.linker.status(), LinkerAccount::SignedOut { .. });
        let on = {
            let state = self.state.lock().unwrap();
            state.saved.enabled || state.tunnel.is_some()
        };
        if signed_out && on {
            self.turn_off(Some(
                "Linker Connect turned off when you signed out of Linker.".into(),
            ));
        } else {
            self.changed();
        }
    }

    /// At start: turns Linker Connect back on when it was on. The handle is for tests; the app needn't wait.
    pub fn start(&self) -> Option<JoinHandle<()>> {
        let enabled = self.state.lock().unwrap().saved.enabled;
        if !enabled || !self.signed_in() {
            return None;
        }
        let connect = self.me.upgrade()?;
        Some(tokio::spawn(async move { connect.turn_on().await }))
    }

    /// Stops the tunnel as the app quits, leaving Linker Connect on for the next start.
    pub fn shutdown(&self) {
        self.state.lock().unwrap().stop_tunnel();
    }

    pub async fn set_enabled(&self, enabled: bool) {
        if enabled {
            self.turn_on().await;
        } else {
            self.turn_off(None);
        }
    }

    /// Looks up your linked computers again, for the settings page.
    pub async fn refresh(&self) {
        if !self.signed_in() {
            return;
        }
        if let Err(error) = self.list_computers().await {
            self.set_problem(Some(error.to_string()));
        }
        self.changed();
    }

    /// Unlinks one of your computers at Linker; for this one, also turns Linker Connect off here.
    pub async fn unlink(&self, computer_id: &str) {
        let route = format!("/{}", encode_uri_component(computer_id));
        if let Err(error) = self.relay("DELETE", &route, None).await {
            // Already gone is what was asked.
            let gone = error
                .downcast_ref::<RelayError>()
                .is_some_and(|relay| relay.status == 404);
            if !gone {
                self.set_problem(Some(error.to_string()));
                self.changed();
                return;
            }
        }
        let this_one = {
            let mut state = self.state.lock().unwrap();
            let this_one = state.saved.computer_id == computer_id;
            if this_one {
                state.saved.url = None;
                state.saved.tunnel_token = None;
            }
            this_one
        };
        if this_one {
            self.turn_off(None);
        }
        {
            let mut state = self.state.lock().unwrap();
            state.at_limit = false;
            state.problem = None;
        }
        let _ = self.list_computers().await;
        self.changed();
    }

    pub fn sign_out_device(&self, device_id: &str) {
        self.sessions.revoke(device_id);
    }

    /// This computer's public address host while Linker Connect is on, for remote::request_source.
    pub fn host(&self) -> Option<String> {
        let state = self.state.lock().unwrap();
        state.tunnel.as_ref()?;
        let url = Url::parse(state.saved.url.as_deref()?).ok()?;
        let host = url.host_str()?.to_lowercase();
        Some(match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host,
        })
    }

    fn is_off(&self) -> bool {
        let state = self.state.lock().unwrap();
        state.tunnel.is_none() || state.saved.owner_id.is_none()
    }

    /// POST /api/connect/session through the tunnel: someone shows a Linker token. Only the account that linked this
    /// computer gets a one-time code for a session.
    pub async fn admit(&self, token: &str, device: &str) -> Result<String, Refusal> {
        if self.is_off() {
            return Err(refuse(503, "Linker Connect is off on this computer."));
        }
        let now = self.now();
        if !self.state.lock().unwrap().admitting.allow(now) {
            return Err(refuse(429, "Too many tries. Wait a minute."));
        }
        let who = self.linker.who_is(token).await.map_err(|_| {
            refuse(
                502,
                format!(
                    "{} couldn't reach Linker to check your sign-in. Try again.",
                    self.name
                ),
            )
        })?;
        let Some(who) = who else {
            return Err(refuse(
                401,
                "Linker didn't accept that sign-in. Sign out of Linker and in again.",
            ));
        };
        let owner = self.state.lock().unwrap().saved.owner_id.clone();
        if owner.as_deref() != Some(who.as_str())
            || self.linker.user_id().as_deref() != Some(who.as_str())
        {
            return Err(refuse(
                403,
                format!(
                    "{} belongs to another Linker account. Sign in with the same account on both computers.",
                    self.name
                ),
            ));
        }
        Ok(self.sessions.issue_code(device))
    }

    /// POST /api/connect/native: a native app (the iPad app) swaps the one-time code admit gave it for a session it
    /// sends as a Bearer token. The code only exists on this computer, so it can't be used at another.
    pub fn redeem(&self, code: &str) -> Result<Redeemed, Refusal> {
        if self.is_off() {
            return Err(refuse(503, "Linker Connect is off on this computer."));
        }
        let now = self.now();
        if !self.state.lock().unwrap().redeeming.allow(now) {
            return Err(refuse(429, "Too many tries. Wait a minute."));
        }
        let entered = if code.is_empty() {
            None
        } else {
            self.sessions.enter(code, ConnectDeviceKind::Native)
        };
        let Some(entered) = entered else {
            return Err(refuse(
                401,
                "That code ran out or was used already. Sign in again.",
            ));
        };
        Ok(Redeemed {
            session: entered.token,
            expires_at: iso_date(entered.expires_at),
        })
    }

    /// Your other linked computers, for the computer menu here; empty when signed out or Linker is unreachable.
    pub async fn others(&self) -> Vec<LinkedComputer> {
        if !self.signed_in() {
            return Vec::new();
        }
        let cached = {
            let state = self.state.lock().unwrap();
            state
                .computers
                .clone()
                .filter(|_| self.now() - state.listed_at < LIST_FRESH_FOR)
        };
        let list = match cached {
            Some(list) => list,
            None => match self.list_computers().await {
                Ok(list) => list,
                Err(error) => {
                    tracing::error!("Couldn't list Linker Connect computers: {error}");
                    return Vec::new();
                }
            },
        };
        let own = self.state.lock().unwrap().saved.computer_id.clone();
        list.into_iter()
            .filter(|computer| computer.id != own)
            .collect()
    }

    /// Opening another of your computers from here (POST /api/connect/open): asks it for a one-time code with this
    /// computer's Linker sign-in, and answers the address that swaps the code for a session.
    pub async fn open(&self, url: &str) -> anyhow::Result<String> {
        let origin = Url::parse(url)?.origin().ascii_serialization();
        let find = |list: &[LinkedComputer]| {
            list.iter()
                .find(|computer| {
                    Url::parse(&computer.url)
                        .is_ok_and(|url| url.origin().ascii_serialization() == origin)
                })
                .cloned()
        };
        let cached = self.state.lock().unwrap().computers.clone();
        let computer = match cached.as_deref().and_then(find) {
            Some(computer) => Some(computer),
            None => find(&self.list_computers().await?),
        };
        let Some(computer) = computer else {
            return Err(anyhow!(
                "That computer isn't linked to your Linker account."
            ));
        };
        let Some(token) = self.linker.access_token().await? else {
            return Err(anyhow!("Sign in with Linker on this computer first."));
        };
        let request = Request::new("POST", format!("{origin}/api/connect/session"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .body(json!({ "name": self.name }).to_string())
            .timeout(NETWORK_TIMEOUT);
        let Ok(res) = self.fetch.fetch(request).await else {
            return Err(anyhow!(
                "Couldn't reach {}. Is it on, with Linker-Edit running and Linker Connect on?",
                computer.name
            ));
        };
        let body = res.json_value().unwrap_or(Value::Null);
        match body.get("code").and_then(Value::as_str) {
            Some(code) if res.ok() => Ok(format!(
                "{origin}/api/connect/enter?code={}",
                encode_uri_component(code)
            )),
            _ => Err(anyhow!(match body.get("error").and_then(Value::as_str) {
                Some(error) => error.to_string(),
                None => format!("{} answered {}.", computer.name, res.status),
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::Response;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};

    const APP: &str = "https://app.example";

    /// The Linker account on this computer: Jane, unless signed out.
    struct FakeAccount {
        signed_in: AtomicBool,
        user_id: Mutex<Option<String>>,
        offline: AtomicBool,
        listeners: Listeners,
    }

    impl FakeAccount {
        fn new() -> Arc<Self> {
            Arc::new(FakeAccount {
                signed_in: AtomicBool::new(true),
                user_id: Mutex::new(Some("user_jane".into())),
                offline: AtomicBool::new(false),
                listeners: Listeners::default(),
            })
        }
        fn sign_out(&self) {
            self.signed_in.store(false, Ordering::SeqCst);
            self.listeners.notify();
        }
        fn is_signed_in(&self) -> bool {
            self.signed_in.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl LinkerService for FakeAccount {
        fn status(&self) -> LinkerAccount {
            if self.is_signed_in() {
                LinkerAccount::SignedIn {
                    name: "Jane".into(),
                    email: None,
                    picture: None,
                    plan: None,
                    app_url: APP.into(),
                }
            } else {
                LinkerAccount::SignedOut { problem: None }
            }
        }
        fn on_status(&self, listener: Listener) {
            self.listeners.add(listener);
        }
        async fn access_token(&self) -> anyhow::Result<Option<String>> {
            Ok(self.is_signed_in().then(|| "token-jane".to_string()))
        }
        fn user_id(&self) -> Option<String> {
            self.user_id
                .lock()
                .unwrap()
                .clone()
                .filter(|_| self.is_signed_in())
        }
        async fn refresh(&self) {}
        async fn who_is(&self, token: &str) -> anyhow::Result<Option<String>> {
            if self.offline.load(Ordering::SeqCst) {
                return Err(anyhow!("offline"));
            }
            Ok(match token {
                "token-jane" => Some("user_jane".into()),
                "token-bob" => Some("user_bob".into()),
                _ => None,
            })
        }
    }

    struct FakeComputer {
        shown: LinkedComputer,
        owner: String,
    }

    /// Linker's /api/edit/computers, for Jane, with others' computers counted too; and the other computers' tunnels.
    #[derive(Default)]
    struct FakeRelay {
        computers: Mutex<Vec<FakeComputer>>,
        calls: Mutex<Vec<String>>,
        down: AtomicBool,
    }

    impl FakeRelay {
        fn mine(&self) -> Vec<LinkedComputer> {
            self.computers
                .lock()
                .unwrap()
                .iter()
                .filter(|c| c.owner == "user_jane")
                .map(|c| c.shown.clone())
                .collect()
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
        fn add_other(&self, owner: &str, id: &str) {
            let mut computers = self.computers.lock().unwrap();
            let n = computers.len() + 1;
            computers.push(FakeComputer {
                shown: LinkedComputer {
                    id: id.into(),
                    name: format!("PC {n}"),
                    url: format!("https://edit-{n:016}.linker.example"),
                    online: Some(true),
                    linked_at: "2026-11-15T10:00:00.000Z".into(),
                    last_seen_at: None,
                },
                owner: owner.into(),
            });
        }
    }

    fn reply(status: u16, body: Value) -> anyhow::Result<Response> {
        Ok(Response::json(status, &body))
    }

    #[async_trait]
    impl Fetch for FakeRelay {
        async fn fetch(&self, request: Request) -> anyhow::Result<Response> {
            let url = Url::parse(&request.url)?;
            let method = request.method.clone();
            self.calls.lock().unwrap().push(format!("{method} {url}"));
            if self.down.load(Ordering::SeqCst) {
                return Err(anyhow!("fetch failed"));
            }
            let origin = url.origin().ascii_serialization();
            if origin.starts_with("https://edit-") && url.path() == "/api/connect/session" {
                return if origin == "https://edit-0000000000000002.linker.example" {
                    reply(200, json!({ "code": "one-time" }))
                } else {
                    reply(403, json!({ "error": "Not yours." }))
                };
            }
            if request.header_value("authorization") != Some("Bearer token-jane") {
                return reply(401, json!({ "error": "unauthorized" }));
            }
            let raw = url
                .path()
                .trim_start_matches("/api/edit/computers")
                .trim_start_matches('/');
            let id = percent_encoding::percent_decode_str(raw)
                .decode_utf8_lossy()
                .into_owned();
            match method.as_str() {
                "GET" => reply(200, json!({ "limit": LIMIT, "computers": self.mine() })),
                "PUT" => {
                    let body: Value =
                        serde_json::from_str(request.body.as_deref().unwrap_or("{}"))?;
                    let name = body["name"].as_str().unwrap_or_default().to_string();
                    let mine = self.mine();
                    let mut computers = self.computers.lock().unwrap();
                    let size = computers.len();
                    let existing = computers.iter_mut().find(|c| c.shown.id == id);
                    if existing.as_ref().is_some_and(|c| c.owner != "user_jane") {
                        return reply(409, json!({ "error": "taken" }));
                    }
                    if existing.is_none() && mine.len() >= LIMIT as usize {
                        return reply(
                            409,
                            json!({ "error": "limit", "limit": LIMIT, "computers": mine }),
                        );
                    }
                    let shown = match existing {
                        Some(computer) => {
                            computer.shown.name = name;
                            computer.shown.clone()
                        }
                        None => {
                            let shown = LinkedComputer {
                                id: id.clone(),
                                name,
                                url: format!("https://edit-{:016}.linker.example", size + 1),
                                online: Some(true),
                                linked_at: "2026-11-16T10:00:00.000Z".into(),
                                last_seen_at: None,
                            };
                            computers.push(FakeComputer {
                                shown: shown.clone(),
                                owner: "user_jane".into(),
                            });
                            shown
                        }
                    };
                    reply(
                        200,
                        json!({ "computer": shown, "tunnelToken": format!("tunnel-{id}") }),
                    )
                }
                "DELETE" => {
                    let mut computers = self.computers.lock().unwrap();
                    let before = computers.len();
                    computers.retain(|c| c.shown.id != id);
                    if computers.len() < before {
                        Ok(Response::new(204, ""))
                    } else {
                        reply(404, json!({ "error": "not_found" }))
                    }
                }
                _ => reply(405, json!({ "error": "?" })),
            }
        }
    }

    struct Run {
        token: String,
        stopped: AtomicBool,
        emit: crate::connect::cloudflared::OnTunnelEvent,
    }

    struct RunHandle(Arc<Run>);

    impl Tunnel for RunHandle {
        fn stop(&self) {
            self.0.stopped.store(true, Ordering::SeqCst);
        }
    }

    /// cloudflared that does nothing, with its log lines played by the test.
    #[derive(Default)]
    struct FakeTunnels(Mutex<Vec<Arc<Run>>>);

    impl FakeTunnels {
        fn runs(&self) -> Vec<Arc<Run>> {
            self.0.lock().unwrap().clone()
        }
    }

    impl StartTunnel for FakeTunnels {
        fn start(
            &self,
            _file: &Path,
            token: &str,
            on_event: crate::connect::cloudflared::OnTunnelEvent,
        ) -> Box<dyn Tunnel> {
            let run = Arc::new(Run {
                token: token.into(),
                stopped: AtomicBool::new(false),
                emit: on_event,
            });
            self.0.lock().unwrap().push(run.clone());
            Box::new(RunHandle(run))
        }
    }

    struct FakePrepare;

    #[async_trait]
    impl PrepareCloudflared for FakePrepare {
        async fn prepare(&self, on_percent: OnPercent) -> anyhow::Result<PathBuf> {
            on_percent(100);
            Ok(PathBuf::from("cloudflared"))
        }
    }

    struct Setup {
        dir: tempfile::TempDir,
        linker: Arc<FakeAccount>,
        relay: Arc<FakeRelay>,
        tunnels: Arc<FakeTunnels>,
        sessions: Arc<Sessions>,
    }

    impl Setup {
        fn make(&self) -> Arc<Connect> {
            Connect::new(ConnectOptions {
                file: self.dir.path().join("connect.json"),
                port: 4317,
                linker: self.linker.clone(),
                app_url: APP.into(),
                sessions: self.sessions.clone(),
                prepare: Arc::new(FakePrepare),
                start_tunnel: self.tunnels.clone(),
                fetch: self.relay.clone(),
                name: "Studio".into(),
                now: system_clock(),
            })
        }
    }

    fn setup() -> (Setup, Arc<Connect>) {
        let dir = tempfile::tempdir().unwrap();
        let sessions = Arc::new(Sessions::new(dir.path().join("connect-sessions.json")));
        let setup = Setup {
            dir,
            linker: FakeAccount::new(),
            relay: Arc::default(),
            tunnels: Arc::default(),
            sessions,
        };
        let connect = setup.make();
        (setup, connect)
    }

    fn ids(computers: &[LinkedComputer]) -> Vec<&str> {
        computers.iter().map(|c| c.id.as_str()).collect()
    }

    #[tokio::test]
    async fn links_this_computer_and_runs_its_tunnel() {
        let (setup, connect) = setup();
        assert_eq!(connect.status().state, LinkerConnectState::Off);
        assert_eq!(connect.status().url, None);
        connect.set_enabled(true).await;
        let id = connect.status().computer_id;
        assert!(
            setup
                .relay
                .calls()
                .contains(&format!("PUT {APP}/api/edit/computers/{id}"))
        );
        let runs = setup.tunnels.runs();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].token, format!("tunnel-{id}"));
        let status = connect.status();
        assert_eq!(status.state, LinkerConnectState::On);
        assert!(!status.connected);
        assert_eq!(
            status.url.as_deref(),
            Some("https://edit-0000000000000001.linker.example")
        );
        (runs[0].emit)(TunnelEvent::Up);
        assert!(connect.status().connected);
        assert_eq!(
            connect.host().as_deref(),
            Some("edit-0000000000000001.linker.example")
        );
        // The tunnel token stays in a file only this user reads, and the next start reuses the link.
        let saved = std::fs::read_to_string(setup.dir.path().join("connect.json")).unwrap();
        assert!(saved.contains("tunnel-"));
        let saved: Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(saved["enabled"], json!(true));
        assert_eq!(saved["ownerId"], json!("user_jane"));
    }

    #[tokio::test]
    async fn stops_at_three_computers_until_one_is_unlinked() {
        let (setup, connect) = setup();
        for id in ["a", "b", "c"] {
            setup.relay.add_other("user_jane", id);
        }
        connect.set_enabled(true).await;
        assert!(setup.tunnels.runs().is_empty());
        let status = connect.status();
        assert_eq!(status.state, LinkerConnectState::Off);
        assert!(status.at_limit);
        assert!(status.problem.unwrap().contains("3 computers"));
        assert_eq!(ids(&status.computers.unwrap()), ["a", "b", "c"]);
        connect.unlink("b").await;
        let status = connect.status();
        assert!(!status.at_limit);
        assert_eq!(status.problem, None);
        connect.set_enabled(true).await;
        let status = connect.status();
        assert_eq!(status.state, LinkerConnectState::On);
        assert_eq!(status.computers.unwrap().len(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn restarts_a_tunnel_that_stopped_by_itself_and_not_one_turned_off() {
        let (setup, connect) = setup();
        connect.set_enabled(true).await;
        (setup.tunnels.runs()[0].emit)(TunnelEvent::Down {
            detail: Some("lost".into()),
        });
        let status = connect.status();
        assert!(!status.connected);
        assert_eq!(status.problem.as_deref(), Some("The tunnel stopped: lost"));
        connect.set_enabled(false).await;
        assert_eq!(connect.status().state, LinkerConnectState::Off);
        tokio::time::sleep(Duration::from_millis(2_100)).await;
        assert_eq!(setup.tunnels.runs().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn starts_a_tunnel_that_stopped_by_itself_again() {
        let (setup, connect) = setup();
        connect.set_enabled(true).await;
        (setup.tunnels.runs()[0].emit)(TunnelEvent::Down { detail: None });
        tokio::time::sleep(Duration::from_millis(2_100)).await;
        assert_eq!(setup.tunnels.runs().len(), 2);
        assert_eq!(connect.status().state, LinkerConnectState::On);
    }

    #[tokio::test]
    async fn lets_in_only_the_account_that_linked_it() {
        let (setup, connect) = setup();
        assert_eq!(
            connect
                .admit("token-jane", "Laptop")
                .await
                .unwrap_err()
                .status,
            503
        );
        connect.set_enabled(true).await;
        let code = connect.admit("token-jane", "Laptop").await.unwrap();
        assert!(
            setup
                .sessions
                .enter(&code, ConnectDeviceKind::Cookie)
                .is_some()
        );
        let refused = connect.admit("token-bob", "Bob's").await.unwrap_err();
        assert_eq!(refused.status, 403);
        assert!(refused.error.contains("another Linker account"));
        assert_eq!(connect.admit("stolen", "?").await.unwrap_err().status, 401);
        setup.linker.offline.store(true, Ordering::SeqCst);
        assert_eq!(
            connect
                .admit("token-jane", "Laptop")
                .await
                .unwrap_err()
                .status,
            502
        );
    }

    #[tokio::test]
    async fn gives_the_ipad_app_a_bearer_session_for_a_code_once_and_only_while_on() {
        let (setup, connect) = setup();
        assert_eq!(connect.redeem("anything").unwrap_err().status, 503);
        connect.set_enabled(true).await;
        let code = connect.admit("token-jane", "iPad").await.unwrap();
        let redeemed = connect.redeem(&code).unwrap();
        assert!(chrono::DateTime::parse_from_rfc3339(&redeemed.expires_at).is_ok());
        assert!(redeemed.expires_at.ends_with('Z'));
        assert_eq!(
            serde_json::to_value(&redeemed)
                .unwrap()
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>(),
            ["expiresAt", "session"]
        );
        assert!(
            setup
                .sessions
                .check(Some(&redeemed.session), ConnectDeviceKind::Native)
                .is_some()
        );
        let devices = setup.sessions.list();
        assert_eq!(devices.len(), 1);
        assert_eq!(
            (devices[0].name.as_str(), devices[0].kind),
            ("iPad", ConnectDeviceKind::Native)
        );
        assert_eq!(connect.redeem(&code).unwrap_err().status, 401);
    }

    #[tokio::test]
    async fn allows_only_so_many_tries_a_minute_at_each_way_in() {
        let (_setup, connect) = setup();
        connect.set_enabled(true).await;
        for _ in 0..20 {
            assert_eq!(connect.redeem("guess").unwrap_err().status, 401);
        }
        assert_eq!(connect.redeem("guess").unwrap_err().status, 429);
        // Guessing codes doesn't lock you out of asking for one.
        assert!(connect.admit("token-jane", "iPad").await.is_ok());
    }

    #[tokio::test]
    async fn signs_every_device_out_when_another_linker_account_links_this_computer() {
        let (setup, connect) = setup();
        connect.set_enabled(true).await;
        let native = setup
            .sessions
            .enter(
                &setup.sessions.issue_code("iPad"),
                ConnectDeviceKind::Native,
            )
            .unwrap();
        // Restarted with Bob signed in here: Linker Connect comes back on for him, and drops Jane's devices.
        *setup.linker.user_id.lock().unwrap() = Some("user_bob".into());
        setup.make().start().unwrap().await.unwrap();
        assert_eq!(
            setup
                .sessions
                .check(Some(&native.token), ConnectDeviceKind::Native),
            None
        );
    }

    #[tokio::test]
    async fn turns_off_and_signs_every_device_out_when_linker_is_signed_out() {
        let (setup, connect) = setup();
        connect.set_enabled(true).await;
        let entered = setup
            .sessions
            .enter(
                &setup.sessions.issue_code("Laptop"),
                ConnectDeviceKind::Cookie,
            )
            .unwrap();
        setup.linker.sign_out();
        assert!(setup.tunnels.runs()[0].stopped.load(Ordering::SeqCst));
        assert_eq!(
            setup
                .sessions
                .check(Some(&entered.token), ConnectDeviceKind::Cookie),
            None
        );
        let status = connect.status();
        assert_eq!(status.state, LinkerConnectState::SignedOut);
        assert!(status.problem.unwrap().contains("signed out"));
        assert_eq!(connect.host(), None);
    }

    #[tokio::test]
    async fn comes_back_on_at_start_even_offline_with_a_link_from_before() {
        let (setup, connect) = setup();
        connect.set_enabled(true).await;
        setup.relay.down.store(true, Ordering::SeqCst);
        let again = setup.make();
        again.start().unwrap().await.unwrap();
        assert_eq!(again.status().state, LinkerConnectState::On);
        assert_eq!(setup.tunnels.runs().len(), 2);
    }

    #[tokio::test]
    async fn opens_your_other_computers_with_a_one_time_code_and_lists_only_those() {
        let (setup, connect) = setup();
        connect.set_enabled(true).await;
        setup.relay.add_other("user_jane", "office");
        setup.relay.add_other("user_bob", "bobs");
        assert!(connect.others().await.is_empty());
        // The list is kept for 30 seconds; the settings page asks for a fresh one.
        connect.refresh().await;
        assert_eq!(ids(&connect.others().await), ["office"]);
        assert_eq!(
            connect
                .open("https://edit-0000000000000002.linker.example/")
                .await
                .unwrap(),
            "https://edit-0000000000000002.linker.example/api/connect/enter?code=one-time"
        );
        let error = connect
            .open("https://edit-0000000000000003.linker.example")
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("isn't linked"), "{error}");
    }

    #[tokio::test]
    async fn unlinking_this_computer_turns_it_off_and_forgets_its_address() {
        let (setup, connect) = setup();
        connect.set_enabled(true).await;
        let code = connect.admit("token-jane", "iPad").await.unwrap();
        let session = connect.redeem(&code).unwrap().session;
        connect.unlink(&connect.status().computer_id).await;
        assert!(setup.tunnels.runs()[0].stopped.load(Ordering::SeqCst));
        assert_eq!(
            setup
                .sessions
                .check(Some(&session), ConnectDeviceKind::Native),
            None
        );
        let status = connect.status();
        assert_eq!(status.state, LinkerConnectState::Off);
        assert_eq!(status.url, None);
        assert_eq!(status.computers, Some(vec![]));
    }

    /// axum runs handlers on any thread, so what the http layer awaits must be Send.
    #[allow(dead_code, clippy::let_underscore_future)]
    fn can_be_awaited_in_axum_handlers(
        connect: Arc<Connect>,
        linker: Arc<crate::linker::Linker>,
        remote: Arc<crate::remote::RemoteAccessService>,
    ) {
        fn send<T: Send>(_: T) {}
        send(connect.admit("", ""));
        send(connect.open(""));
        send(connect.others());
        send(connect.set_enabled(true));
        send(connect.unlink(""));
        send(connect.refresh());
        send(linker.start());
        send(linker.finish(&HashMap::new()));
        send(linker.sign_out());
        send(linker.refresh());
        send(remote.set_enabled(true));
        send(remote.refresh());
        send(remote.computers());
    }

    #[test]
    fn reads_the_link_the_node_app_kept() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("connect.json");
        std::fs::write(
            &file,
            r#"{
  "computerId": "4b0c",
  "enabled": true,
  "url": "https://edit-0000000000000001.linker.example",
  "tunnelToken": "tunnel-4b0c",
  "ownerId": "user_jane"
}"#,
        )
        .unwrap();
        let saved = read(&file);
        assert_eq!(saved.computer_id, "4b0c");
        assert!(saved.enabled);
        assert_eq!(saved.owner_id.as_deref(), Some("user_jane"));
        let written: HashMap<String, Value> = serde_json::from_str(&pretty_json(&saved)).unwrap();
        assert_eq!(written.len(), 5);
        assert!(!read(&dir.path().join("missing.json")).enabled);
    }
}
