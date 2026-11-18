// Remote access over Tailscale: serves this app to the owner's other devices with Tailscale Serve, and decides
// where a request comes from (request_source), which the http layer gates every request and /ws upgrade on.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::hash::BuildHasher;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use linker_edit_protocol::{Computer, RemoteAccess};
use serde::de::{Deserialize, Deserializer, MapAccess, Visitor};
use serde_json::Value;
use tokio::io::AsyncReadExt;
use url::Url;

use crate::net::{Listener, Listeners};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailscaleResult {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

/// Runs the tailscale CLI; None when it isn't installed.
#[async_trait]
pub trait RunTailscale: Send + Sync {
    async fn run(&self, args: &[String], timeout: Duration) -> Option<TailscaleResult>;
}

/// Who may come in over Tailscale: the tailnet address this server is served at, and the account that owns this
/// computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteGate {
    /// host[:port] as browsers send it, such as studio.tail1234.ts.net:8443.
    pub host: String,
    pub owner: String,
}

/// The hosts that are this computer itself; http.ts's isAllowedOrigin uses them too.
pub const LOOPBACK_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "[::1]"];
const DOWNLOAD: &str = "https://tailscale.com/download";
const DNS_SETTINGS: &str = "https://login.tailscale.com/admin/dns";

fn tailscale_command() -> PathBuf {
    let installed = if cfg!(windows) {
        let program_files =
            std::env::var_os("PROGRAMFILES").unwrap_or_else(|| "C:\\Program Files".into());
        Some(
            PathBuf::from(program_files)
                .join("Tailscale")
                .join("tailscale.exe"),
        )
    } else if cfg!(target_os = "macos") {
        Some(PathBuf::from(
            "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
        ))
    } else {
        None
    };
    installed
        .filter(|file| file.exists())
        .unwrap_or_else(|| PathBuf::from("tailscale"))
}

/// The windowsHide of Node's child processes.
pub(crate) fn hide_window(command: &mut tokio::process::Command) {
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    #[cfg(not(windows))]
    let _ = command;
}

/// The real tailscale CLI.
#[derive(Debug, Clone, Copy, Default)]
pub struct TailscaleCli;

#[async_trait]
impl RunTailscale for TailscaleCli {
    async fn run(&self, args: &[String], timeout: Duration) -> Option<TailscaleResult> {
        let mut command = tokio::process::Command::new(tailscale_command());
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        hide_window(&mut command);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(_) => {
                return Some(TailscaleResult {
                    code: 1,
                    stdout: String::new(),
                    stderr: String::new(),
                    timed_out: false,
                });
            }
        };
        let read = |pipe: Option<Box<dyn tokio::io::AsyncRead + Send + Unpin>>| {
            tokio::spawn(async move {
                let mut bytes = Vec::new();
                if let Some(mut pipe) = pipe {
                    let _ = pipe.read_to_end(&mut bytes).await;
                }
                String::from_utf8_lossy(&bytes).into_owned()
            })
        };
        let stdout = read(child.stdout.take().map(|pipe| Box::new(pipe) as _));
        let stderr = read(child.stderr.take().map(|pipe| Box::new(pipe) as _));
        let (code, timed_out) = match tokio::time::timeout(timeout, child.wait()).await {
            Ok(Ok(status)) => (status.code().unwrap_or(1), false),
            Ok(Err(_)) => (1, false),
            Err(_) => {
                let _ = child.kill().await;
                (1, true)
            }
        };
        // What it printed before it stopped, such as the link `tailscale serve` waits on.
        Some(TailscaleResult {
            code,
            stdout: stdout.await.unwrap_or_default(),
            stderr: stderr.await.unwrap_or_default(),
            timed_out,
        })
    }
}

/// A request's headers by lowercase name, however the http layer holds them.
pub trait Headers {
    /// The header's value; Some("") when it was sent empty, None when it wasn't sent or isn't text.
    fn header(&self, name: &str) -> Option<&str>;
    /// Whether the header was sent at all.
    fn has_header(&self, name: &str) -> bool {
        self.header(name).is_some()
    }
}

impl Headers for http::HeaderMap {
    fn header(&self, name: &str) -> Option<&str> {
        self.get(name).and_then(|value| value.to_str().ok())
    }
    fn has_header(&self, name: &str) -> bool {
        self.contains_key(name)
    }
}

impl<S: BuildHasher> Headers for HashMap<String, String, S> {
    fn header(&self, name: &str) -> Option<&str> {
        self.get(name).map(String::as_str)
    }
}

impl Headers for BTreeMap<String, String> {
    fn header(&self, name: &str) -> Option<&str> {
        self.get(name).map(String::as_str)
    }
}

/// `new URL("https://" + value)`: its host (with a port that isn't 443) and hostname, lowercase.
fn parse_host(value: &str) -> Option<(String, String)> {
    if value.is_empty() {
        return None;
    }
    let url = Url::parse(&format!("https://{value}")).ok()?;
    let hostname = url.host_str()?.to_lowercase();
    let host = match url.port() {
        Some(port) => format!("{hostname}:{port}"),
        None => hostname.clone(),
    };
    Some((host, hostname))
}

fn host_of(value: Option<&str>) -> Option<String> {
    parse_host(value?).map(|(host, _)| host)
}

/// Where a request comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestSource {
    /// This computer itself.
    Local,
    /// The owner's devices through Tailscale Serve.
    Remote,
    /// The Linker Connect tunnel, which still needs a session (connect::sessions::Sessions::session_of).
    Connect,
}

/// Where a request comes from: this computer, the owner's devices through Tailscale Serve, the Linker Connect
/// tunnel (which still needs a session), or neither (None: refuse it with 403).
/// Tailscale Serve proxies from 127.0.0.1, keeps the tailnet Host, adds X-Forwarded-For and sets the
/// Tailscale-User-Login of the device's account after dropping any the client sent.
/// connect_host is connect::Connect::host(): this computer's tunnel address while Linker Connect is on.
pub fn request_source(
    headers: &(impl Headers + ?Sized),
    gate: Option<&RemoteGate>,
    connect_host: Option<&str>,
) -> Option<RequestSource> {
    let parsed = headers.header("host").and_then(parse_host);
    let host = parsed.as_ref().map(|(host, _)| host.as_str());
    let hostname = parsed.as_ref().map(|(_, hostname)| hostname.as_str());
    let loopback = hostname.is_some_and(|name| LOOPBACK_HOSTS.contains(&name));
    // Tailscale Serve and Cloudflare's tunnel (cloudflared) both say who they forward for.
    let proxied = [
        "x-forwarded-for",
        "tailscale-user-login",
        "cf-connecting-ip",
        "cf-ray",
    ]
    .iter()
    .any(|name| headers.has_header(name));
    if loopback && !proxied {
        return Some(RequestSource::Local);
    }
    if !proxied {
        return None;
    }
    // Should a proxy ever stop keeping the Host, the forwarded one still names the public address.
    let public_host = if loopback {
        host_of(Some(headers.header("x-forwarded-host").unwrap_or("")))
    } else {
        host.map(str::to_string)
    };
    // Linker Connect: through the tunnel, to this computer's address at Linker. A session is checked after this.
    // Cloudflare sets CF-Connecting-IP itself. Whatever else such a request claims, it never counts as Tailscale.
    if headers.has_header("cf-connecting-ip") {
        return match (connect_host, public_host) {
            (Some(connect), Some(public)) if public == connect.to_lowercase() => {
                Some(RequestSource::Connect)
            }
            _ => None,
        };
    }
    let gate = gate?;
    let owner = headers
        .header("tailscale-user-login")
        .is_some_and(|login| login.to_lowercase() == gate.owner.to_lowercase());
    (public_host.is_some() && public_host == host_of(Some(&gate.host)) && owner)
        .then_some(RequestSource::Remote)
}

pub fn remote_url(host: &str, serve_port: u16) -> String {
    if serve_port == 443 {
        format!("https://{host}")
    } else {
        format!("https://{host}:{serve_port}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tailnet {
    Ok {
        host: String,
        owner: String,
        https_certs: bool,
    },
    Missing {
        hint: String,
    },
}

/// A JSON object's entries in the order they were written, as Object.values sees them.
struct Ordered(Vec<(String, Value)>);

impl<'de> Deserialize<'de> for Ordered {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Entries;
        impl<'de> Visitor<'de> for Entries {
            type Value = Ordered;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Ordered, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(Ordered(entries))
            }
            fn visit_unit<E>(self) -> Result<Ordered, E> {
                Ok(Ordered(Vec::new()))
            }
        }
        deserializer.deserialize_any(Entries)
    }
}

fn dns_host(value: Option<&Value>) -> Option<String> {
    let name = value?.as_str()?;
    let host = name.strip_suffix('.').unwrap_or(name).to_lowercase();
    (!host.is_empty()).then_some(host)
}

/// JavaScript's ===, for the numbers in Tailscale's JSON.
fn same_number(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(a), Some(b)) => a == b,
        _ => a == b,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub name: String,
    pub host: String,
}

/// The user's own devices that are online now, by MagicDNS name. Other people's shared devices are left out.
pub fn own_peers(status_json: &str) -> Vec<Peer> {
    let Ok(status) = serde_json::from_str::<Value>(status_json) else {
        return Vec::new();
    };
    let Some(user_id) = status.pointer("/Self/UserID").filter(|id| !id.is_null()) else {
        return Vec::new();
    };
    if status.get("BackendState").and_then(Value::as_str) != Some("Running") {
        return Vec::new();
    }
    #[derive(serde::Deserialize)]
    struct PeersOnly {
        #[serde(rename = "Peer")]
        peer: Option<Ordered>,
    }
    let peers = serde_json::from_str::<PeersOnly>(status_json)
        .ok()
        .and_then(|status| status.peer)
        .map(|peers| peers.0)
        .unwrap_or_default();
    peers
        .into_iter()
        .filter_map(|(_, peer)| {
            let host = dns_host(peer.get("DNSName"))?;
            let same_user = peer
                .get("UserID")
                .is_some_and(|id| same_number(id, user_id));
            let online = peer.get("Online").and_then(Value::as_bool) == Some(true);
            if !same_user || !online {
                return None;
            }
            let name = peer
                .get("HostName")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| host.split('.').next().unwrap_or_default().to_string());
            Some(Peer { name, host })
        })
        .collect()
}

/// What Linker-Edit answers at /api/hello.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    pub name: String,
    pub version: String,
}

/// Linker-Edit's answer at a remote address, or None when nothing (or something else) answers.
#[async_trait]
pub trait Probe: Send + Sync {
    async fn probe(&self, url: &str) -> Option<Hello>;
}

/// The real Probe: GET {url}/api/hello, for 5 seconds at most.
#[derive(Debug, Clone, Copy, Default)]
pub struct LinkerEditProbe;

#[async_trait]
impl Probe for LinkerEditProbe {
    async fn probe(&self, url: &str) -> Option<Hello> {
        probe_linker_edit(url).await
    }
}

pub async fn probe_linker_edit(url: &str) -> Option<Hello> {
    let response = reqwest::Client::new()
        .get(format!("{url}/api/hello"))
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .ok()?;
    let ok = response.status().is_success();
    let hello = response.json::<Value>().await.ok()?;
    let text = |key: &str| {
        hello
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    (ok && hello.get("app").and_then(Value::as_str) == Some("linker-edit")).then(|| Hello {
        name: text("name"),
        version: text("version"),
    })
}

pub fn parse_status(result: Option<&TailscaleResult>) -> Tailnet {
    let missing = |hint: String| Tailnet::Missing { hint };
    let Some(result) = result else {
        return missing(format!(
            "Install Tailscale on this computer and the other one, and log in to the same account: {DOWNLOAD}"
        ));
    };
    let Ok(status) = serde_json::from_str::<Value>(&result.stdout) else {
        return missing("Start Tailscale on this computer.".into());
    };
    match status.get("BackendState").and_then(Value::as_str) {
        Some("Running") => {}
        Some("NeedsLogin" | "NoState") => {
            return missing("Log in to Tailscale on this computer.".into());
        }
        Some("NeedsMachineAuth") => {
            return missing(
                "This computer is waiting for approval in the Tailscale admin console.".into(),
            );
        }
        Some("Stopped") => {
            return missing("Tailscale is turned off on this computer. Turn it on.".into());
        }
        _ => return missing("Tailscale is starting…".into()),
    }
    let Some(host) = dns_host(status.pointer("/Self/DNSName")) else {
        return missing(format!(
            "Turn on MagicDNS in the Tailscale admin console: {DNS_SETTINGS}"
        ));
    };
    // String(status.Self?.UserID): "7" for 7, "undefined" when it's missing.
    let user_key = match status.pointer("/Self/UserID") {
        None => "undefined".to_string(),
        Some(Value::String(id)) => id.clone(),
        Some(id) => id.to_string(),
    };
    let owner = status
        .get("User")
        .and_then(|users| users.get(&user_key))
        .and_then(|user| user.get("LoginName"))
        .and_then(Value::as_str)
        .filter(|login| !login.is_empty());
    let Some(owner) = owner else {
        return missing("Tailscale doesn't say which account this computer belongs to. Log in to Tailscale with your own account.".into());
    };
    // Older CLIs report no CapMap; serve will say so itself if certificates are off.
    let https_certs = match status.pointer("/Self/CapMap") {
        Some(Value::Object(caps)) => caps.contains_key("https"),
        _ => true,
    };
    Tailnet::Ok {
        host,
        owner: owner.to_string(),
        https_certs,
    }
}

/// What Tailscale Serve sends the HTTPS port to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Served {
    This,
    Nothing,
    Other,
}

fn normalize_target(target: &str) -> String {
    let has_scheme = target.split_once("://").is_some_and(|(scheme, _)| {
        !scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_alphabetic())
    });
    let target = if has_scheme {
        target.to_string()
    } else {
        format!("http://{target}")
    };
    target.strip_suffix('/').unwrap_or(&target).to_string()
}

/// JavaScript truthiness of a JSON value.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(_) => true,
    }
}

/// What Tailscale Serve sends the HTTPS port to: this server, nothing, or something else.
pub fn served_on(serve_json: &str, host: &str, serve_port: u16, target: &str) -> Served {
    let text = if serve_json.is_empty() {
        "{}"
    } else {
        serve_json
    };
    let Ok(serve) = serde_json::from_str::<Value>(text) else {
        return Served::Nothing;
    };
    let proxy = serve
        .get("Web")
        .and_then(|web| web.get(format!("{host}:{serve_port}")))
        .and_then(|site| site.get("Handlers"))
        .and_then(|handlers| handlers.get("/"))
        .and_then(|handler| handler.get("Proxy"));
    if let Some(proxy) = proxy.and_then(Value::as_str).filter(|p| !p.is_empty())
        && normalize_target(proxy) == normalize_target(target)
    {
        return Served::This;
    }
    let tcp = serve
        .get("TCP")
        .and_then(|tcp| tcp.get(serve_port.to_string()));
    if truthy(proxy) || truthy(tcp) {
        Served::Other
    } else {
        Served::Nothing
    }
}

fn last_lines(text: &str) -> String {
    let lines: Vec<&str> = text.trim().lines().collect();
    lines[lines.len().saturating_sub(4)..]
        .join(" ")
        .trim()
        .to_string()
}

pub struct RemoteAccessOptions {
    /// This server's port on 127.0.0.1.
    pub port: u16,
    /// The HTTPS port on the tailnet (AppConfig::tailscale_port).
    pub serve_port: u16,
    pub run: Arc<dyn RunTailscale>,
    pub probe: Arc<dyn Probe>,
}

impl RemoteAccessOptions {
    /// The real tailscale CLI and /api/hello.
    pub fn new(port: u16, serve_port: u16) -> Self {
        RemoteAccessOptions {
            port,
            serve_port,
            run: Arc::new(TailscaleCli),
            probe: Arc::new(LinkerEditProbe),
        }
    }
}

struct State {
    status: RemoteAccess,
    gate: Option<RemoteGate>,
    switching: bool,
    /// Never take over a port another Serve setting uses.
    port_taken: bool,
}

/// Serves this app to the owner's other devices with Tailscale Serve. Tailscale keeps that setting across restarts.
pub struct RemoteAccessService {
    options: RemoteAccessOptions,
    target: String,
    state: Mutex<State>,
    listeners: Listeners,
}

impl RemoteAccessService {
    pub fn new(options: RemoteAccessOptions) -> Self {
        RemoteAccessService {
            target: format!("http://127.0.0.1:{}", options.port),
            options,
            state: Mutex::new(State {
                status: RemoteAccess::Checking,
                gate: None,
                switching: false,
                port_taken: false,
            }),
            listeners: Listeners::default(),
        }
    }

    pub fn status(&self) -> RemoteAccess {
        self.state.lock().unwrap().status.clone()
    }

    pub fn on_status(&self, listener: Listener) {
        self.listeners.add(listener);
    }

    /// Lets the user's own devices in through Tailscale Serve while remote access is on; for request_source.
    pub fn gate(&self) -> Option<RemoteGate> {
        self.state.lock().unwrap().gate.clone()
    }

    fn set(&self, status: RemoteAccess, gate: Option<RemoteGate>) {
        {
            let mut state = self.state.lock().unwrap();
            state.status = status;
            state.gate = gate;
        }
        self.listeners.notify();
    }

    async fn read(&self) -> (Tailnet, Served) {
        let status = self
            .options
            .run
            .run(&args(&["status", "--json"]), Duration::from_secs(10))
            .await;
        let tailnet = parse_status(status.as_ref());
        let Tailnet::Ok { host, .. } = &tailnet else {
            return (tailnet, Served::Nothing);
        };
        let serve = self
            .options
            .run
            .run(
                &args(&["serve", "status", "--json"]),
                Duration::from_secs(10),
            )
            .await;
        let served = served_on(
            serve.as_ref().map(|s| s.stdout.as_str()).unwrap_or(""),
            host,
            self.options.serve_port,
            &self.target,
        );
        (tailnet, served)
    }

    /// Reads Tailscale's state; problem is kept on screen when remote access is still off.
    async fn refresh_with(&self, problem: Option<String>) {
        let (tailnet, served) = self.read().await;
        let port_taken = served == Served::Other;
        self.state.lock().unwrap().port_taken = port_taken;
        let (host, owner, https_certs) = match tailnet {
            Tailnet::Missing { hint } => return self.set(RemoteAccess::Unavailable { hint }, None),
            Tailnet::Ok {
                host,
                owner,
                https_certs,
            } => (host, owner, https_certs),
        };
        let url = remote_url(&host, self.options.serve_port);
        if served == Served::This {
            let gate = RemoteGate {
                host: host_of(url.strip_prefix("https://")).unwrap_or_default(),
                owner: owner.clone(),
            };
            return self.set(RemoteAccess::On { url, owner }, Some(gate));
        }
        let conflict = port_taken.then(|| format!(
            "Tailscale already serves something else on port {}. Set \"tailscalePort\" in linker-edit.config.json to another port.",
            self.options.serve_port
        ));
        let certs = (!https_certs).then(|| {
            format!("Turn on HTTPS certificates in the Tailscale admin console: {DNS_SETTINGS}")
        });
        self.set(
            RemoteAccess::Off {
                url,
                problem: conflict.or(problem).or(certs),
            },
            None,
        );
    }

    /// Reads Tailscale's state again; does nothing while switching.
    pub async fn refresh(&self) {
        if self.state.lock().unwrap().switching {
            return;
        }
        self.refresh_with(None).await;
    }

    pub async fn set_enabled(&self, enabled: bool) {
        {
            let mut state = self.state.lock().unwrap();
            let url = match &state.status {
                RemoteAccess::Off { url, .. } if enabled && !state.port_taken => url.clone(),
                RemoteAccess::On { url, .. } if !enabled => url.clone(),
                _ => return,
            };
            if state.switching {
                return;
            }
            state.switching = true;
            state.status = RemoteAccess::Switching { url };
        };
        self.listeners.notify();
        let serve_port = format!("--https={}", self.options.serve_port);
        let command = if enabled {
            args(&["serve", "--bg", &serve_port, &self.target])
        } else {
            args(&["serve", &serve_port, "off"])
        };
        let result = self
            .options
            .run
            .run(&command, Duration::from_secs(30))
            .await;
        let problem = match result {
            None => Some(format!("Tailscale isn't installed: {DOWNLOAD}")),
            Some(result) if result.timed_out => Some(format!(
                "Tailscale is waiting for you. {}",
                last_lines(&(result.stdout + &result.stderr))
            )),
            Some(result) if result.code != 0 => {
                let output = if result.stderr.is_empty() {
                    &result.stdout
                } else {
                    &result.stderr
                };
                let lines = last_lines(output);
                Some(if lines.is_empty() {
                    format!("tailscale serve failed with code {}.", result.code)
                } else {
                    lines
                })
            }
            Some(_) => None,
        };
        self.refresh_with(problem).await;
        self.state.lock().unwrap().switching = false;
    }

    /// The user's other computers where Linker-Edit answers with remote access on.
    pub async fn computers(&self) -> Vec<Computer> {
        let status = self
            .options
            .run
            .run(&args(&["status", "--json"]), Duration::from_secs(10))
            .await;
        let peers = own_peers(status.as_ref().map(|s| s.stdout.as_str()).unwrap_or(""));
        let found = futures::future::join_all(peers.into_iter().map(|peer| async move {
            let url = remote_url(&peer.host, self.options.serve_port);
            let hello = self.options.probe.probe(&url).await?;
            Some(Computer {
                name: if hello.name.is_empty() {
                    peer.name
                } else {
                    hello.name
                },
                url,
                version: hello.version,
                via: None,
                online: None,
            })
        }))
        .await;
        found.into_iter().flatten().collect()
    }
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|arg| arg.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    /// A named request, the gate or tunnel it meets, and where it should count as coming from.
    type Case<T> = (
        &'static str,
        HashMap<String, String>,
        T,
        Option<RequestSource>,
    );

    fn gate() -> RemoteGate {
        RemoteGate {
            host: "studio.tail1234.ts.net:8443".into(),
            owner: "Jane@example.com".into(),
        }
    }

    const VIA_SERVE: [(&str, &str); 2] = [
        ("x-forwarded-for", "100.101.102.103"),
        ("tailscale-user-login", "jane@example.com"),
    ];

    fn with(
        base: &[(&str, &str)],
        extra: &[(&'static str, &'static str)],
    ) -> HashMap<String, String> {
        let mut map = headers(extra);
        map.extend(headers(base));
        map
    }

    #[test]
    fn request_source_over_tailscale() {
        use RequestSource::*;
        let serve = |host: &'static str| with(&[("host", host)], &VIA_SERVE);
        let cases: Vec<Case<Option<RemoteGate>>> = vec![
            (
                "this computer",
                headers(&[("host", "127.0.0.1:4317")]),
                Some(gate()),
                Some(Local),
            ),
            (
                "the dev server",
                headers(&[("host", "localhost:5317")]),
                None,
                Some(Local),
            ),
            (
                "the owner over Serve",
                serve("studio.tail1234.ts.net:8443"),
                Some(gate()),
                Some(Remote),
            ),
            (
                "the owner, with Serve's forwarded host",
                with(
                    &[
                        ("host", "127.0.0.1:4317"),
                        ("x-forwarded-host", "studio.tail1234.ts.net:8443"),
                    ],
                    &VIA_SERVE,
                ),
                Some(gate()),
                Some(Remote),
            ),
            (
                "the owner while remote access is off",
                serve("studio.tail1234.ts.net:8443"),
                None,
                None,
            ),
            (
                "another account",
                with(
                    &[
                        ("host", "studio.tail1234.ts.net:8443"),
                        ("tailscale-user-login", "bob@example.com"),
                    ],
                    &VIA_SERVE,
                ),
                Some(gate()),
                None,
            ),
            (
                "a tagged device, which has no login",
                headers(&[
                    ("host", "studio.tail1234.ts.net:8443"),
                    ("x-forwarded-for", "100.64.0.9"),
                ]),
                Some(gate()),
                None,
            ),
            (
                "a proxied request that claims to be local",
                serve("127.0.0.1:4317"),
                Some(gate()),
                None,
            ),
            (
                "another Serve port",
                serve("studio.tail1234.ts.net"),
                Some(gate()),
                None,
            ),
            (
                "a rebound site",
                headers(&[("host", "evil.example:4317")]),
                Some(gate()),
                None,
            ),
            (
                "a rebound site faking Serve",
                serve("evil.example:4317"),
                Some(gate()),
                None,
            ),
            ("no Host", headers(&[]), Some(gate()), None),
        ];
        for (name, headers, current, expected) in cases {
            assert_eq!(
                request_source(&headers, current.as_ref(), None),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn request_source_through_the_tunnel() {
        use RequestSource::*;
        let tunnel = "edit-0123456789abcdef.linker.cool";
        let via_cloudflare: [(&'static str, &'static str); 3] = [
            ("cf-connecting-ip", "203.0.113.9"),
            ("x-forwarded-for", "203.0.113.9"),
            ("cf-ray", "8f00-AMS"),
        ];
        let cases: Vec<Case<Option<&str>>> = vec![
            (
                "the Linker Connect tunnel",
                with(&[("host", tunnel)], &via_cloudflare),
                Some(tunnel),
                Some(Connect),
            ),
            (
                "the tunnel while Linker Connect is off",
                with(&[("host", tunnel)], &via_cloudflare),
                None,
                None,
            ),
            (
                "another address at Cloudflare",
                with(
                    &[("host", "edit-ffffffffffffffff.linker.cool")],
                    &via_cloudflare,
                ),
                Some(tunnel),
                None,
            ),
            (
                "the tunnel faking Tailscale's owner",
                with(
                    &[
                        ("host", "studio.tail1234.ts.net:8443"),
                        ("tailscale-user-login", "jane@example.com"),
                    ],
                    &via_cloudflare,
                ),
                Some(tunnel),
                None,
            ),
            (
                "the tunnel claiming to be local",
                with(&[("host", "127.0.0.1:4317")], &via_cloudflare),
                Some(tunnel),
                None,
            ),
            (
                "Tailscale's owner while Linker Connect is on",
                with(&[("host", "studio.tail1234.ts.net:8443")], &VIA_SERVE),
                Some(tunnel),
                Some(Remote),
            ),
            (
                "this computer while Linker Connect is on",
                headers(&[("host", "127.0.0.1:4317")]),
                Some(tunnel),
                Some(Local),
            ),
        ];
        for (name, headers, connect_host, expected) in cases {
            assert_eq!(
                request_source(&headers, Some(&gate()), connect_host),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn request_source_takes_axum_headers() {
        let mut map = http::HeaderMap::new();
        map.insert("host", "studio.tail1234.ts.net:8443".parse().unwrap());
        map.insert("x-forwarded-for", "100.101.102.103".parse().unwrap());
        map.insert("Tailscale-User-Login", "jane@example.com".parse().unwrap());
        assert_eq!(
            request_source(&map, Some(&gate()), None),
            Some(RequestSource::Remote)
        );
    }

    fn ok(stdout: &str) -> Option<TailscaleResult> {
        Some(TailscaleResult {
            code: 0,
            stdout: stdout.into(),
            stderr: String::new(),
            timed_out: false,
        })
    }

    fn running_status() -> String {
        serde_json::json!({
            "BackendState": "Running",
            "Self": { "DNSName": "Studio.tail1234.ts.net.", "UserID": 7, "CapMap": { "https": null } },
            "User": { "7": { "LoginName": "jane@example.com" } },
        })
        .to_string()
    }

    /// A tailscale CLI that keeps its Serve config, and records what it was asked.
    struct FakeTailscale {
        calls: Mutex<Vec<String>>,
        config: Mutex<Value>,
        status: String,
    }

    impl FakeTailscale {
        fn new(serve: Value) -> Arc<Self> {
            Arc::new(FakeTailscale {
                calls: Mutex::new(Vec::new()),
                config: Mutex::new(serve),
                status: running_status(),
            })
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl RunTailscale for FakeTailscale {
        async fn run(&self, args: &[String], _timeout: Duration) -> Option<TailscaleResult> {
            let joined = args.join(" ");
            self.calls.lock().unwrap().push(joined.clone());
            if args[0] == "status" {
                return ok(&self.status);
            }
            if joined == "serve status --json" {
                return ok(&self.config.lock().unwrap().to_string());
            }
            *self.config.lock().unwrap() = if args.iter().any(|arg| arg == "off") {
                serde_json::json!({})
            } else {
                serde_json::json!({ "Web": { "studio.tail1234.ts.net:8443": { "Handlers": { "/": { "Proxy": args.last().unwrap() } } } } })
            };
            ok("")
        }
    }

    /// Answers every call with the same result.
    struct Always(Option<TailscaleResult>);

    #[async_trait]
    impl RunTailscale for Always {
        async fn run(&self, _args: &[String], _timeout: Duration) -> Option<TailscaleResult> {
            self.0.clone()
        }
    }

    struct NoProbe;

    #[async_trait]
    impl Probe for NoProbe {
        async fn probe(&self, _url: &str) -> Option<Hello> {
            None
        }
    }

    fn remote(run: Arc<dyn RunTailscale>) -> RemoteAccessService {
        RemoteAccessService::new(RemoteAccessOptions {
            port: 4317,
            serve_port: 8443,
            run,
            probe: Arc::new(NoProbe),
        })
    }

    fn problem(status: RemoteAccess) -> Option<String> {
        match status {
            RemoteAccess::Off { problem, .. } => problem,
            other => panic!("not off: {other:?}"),
        }
    }

    #[tokio::test]
    async fn turns_tailscale_serve_on_and_off_for_this_server() {
        let tailscale = FakeTailscale::new(serde_json::json!({}));
        let remote = remote(tailscale.clone());
        remote.refresh().await;
        assert_eq!(
            remote.status(),
            RemoteAccess::Off {
                url: "https://studio.tail1234.ts.net:8443".into(),
                problem: None
            }
        );
        assert_eq!(remote.gate(), None);

        remote.set_enabled(true).await;
        assert!(
            tailscale
                .calls()
                .contains(&"serve --bg --https=8443 http://127.0.0.1:4317".to_string())
        );
        assert_eq!(
            remote.status(),
            RemoteAccess::On {
                url: "https://studio.tail1234.ts.net:8443".into(),
                owner: "jane@example.com".into()
            }
        );
        assert_eq!(
            remote.gate(),
            Some(RemoteGate {
                host: "studio.tail1234.ts.net:8443".into(),
                owner: "jane@example.com".into()
            })
        );

        remote.set_enabled(false).await;
        assert!(
            tailscale
                .calls()
                .contains(&"serve --https=8443 off".to_string())
        );
        assert!(matches!(remote.status(), RemoteAccess::Off { .. }));
        assert_eq!(remote.gate(), None);
    }

    #[tokio::test]
    async fn finds_it_already_on_after_a_restart_as_tailscale_keeps_serve_on() {
        let tailscale = FakeTailscale::new(serde_json::json!({
            "Web": { "studio.tail1234.ts.net:8443": { "Handlers": { "/": { "Proxy": "127.0.0.1:4317" } } } }
        }));
        let remote = remote(tailscale);
        remote.refresh().await;
        assert!(matches!(remote.status(), RemoteAccess::On { .. }));
    }

    #[tokio::test]
    async fn never_takes_over_a_port_that_serves_something_else() {
        let tailscale = FakeTailscale::new(serde_json::json!({
            "Web": { "studio.tail1234.ts.net:8443": { "Handlers": { "/": { "Proxy": "http://127.0.0.1:3000" } } } }
        }));
        let remote = remote(tailscale.clone());
        remote.refresh().await;
        assert!(
            problem(remote.status())
                .unwrap()
                .contains("Set \"tailscalePort\"")
        );
        remote.set_enabled(true).await;
        assert!(
            !tailscale
                .calls()
                .iter()
                .any(|call| call.starts_with("serve --bg"))
        );
    }

    #[tokio::test]
    async fn says_what_to_do_when_tailscale_is_missing() {
        let cases = [
            ("isn't installed", None, "Install Tailscale"),
            (
                "is logged out",
                ok(r#"{"BackendState":"NeedsLogin"}"#),
                "Log in to Tailscale",
            ),
            (
                "isn't running",
                Some(TailscaleResult {
                    code: 1,
                    stdout: String::new(),
                    stderr: "failed to connect to local Tailscale service".into(),
                    timed_out: false,
                }),
                "Start Tailscale",
            ),
        ];
        for (name, result, hint) in cases {
            let remote = remote(Arc::new(Always(result)));
            remote.refresh().await;
            match remote.status() {
                RemoteAccess::Unavailable { hint: shown } => {
                    assert!(shown.contains(hint), "{name}: {shown}")
                }
                other => panic!("{name}: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn finds_only_the_users_own_online_computers_that_run_linker_edit() {
        let status = serde_json::json!({
            "BackendState": "Running",
            "Self": { "DNSName": "laptop.tail1234.ts.net.", "UserID": 7 },
            "Peer": {
                "a": { "HostName": "STUDIO", "DNSName": "studio.tail1234.ts.net.", "UserID": 7, "Online": true },
                "b": { "HostName": "nas", "DNSName": "nas.tail1234.ts.net.", "UserID": 7, "Online": true },
                "c": { "HostName": "old-pc", "DNSName": "old-pc.tail1234.ts.net.", "UserID": 7, "Online": false },
                "d": { "HostName": "friend", "DNSName": "friend.tail1234.ts.net.", "UserID": 8, "Online": true },
            },
        })
        .to_string();
        struct Probed(Mutex<Vec<String>>);
        #[async_trait]
        impl Probe for Probed {
            async fn probe(&self, url: &str) -> Option<Hello> {
                self.0.lock().unwrap().push(url.to_string());
                url.contains("studio").then(|| Hello {
                    name: "STUDIO".into(),
                    version: "2026.1110.0".into(),
                })
            }
        }
        let probed = Arc::new(Probed(Mutex::new(Vec::new())));
        let remote = RemoteAccessService::new(RemoteAccessOptions {
            port: 4317,
            serve_port: 8443,
            run: Arc::new(Always(ok(&status))),
            probe: probed.clone(),
        });
        assert_eq!(
            remote.computers().await,
            vec![Computer {
                name: "STUDIO".into(),
                url: "https://studio.tail1234.ts.net:8443".into(),
                version: "2026.1110.0".into(),
                via: None,
                online: None,
            }]
        );
        assert_eq!(
            *probed.0.lock().unwrap(),
            vec![
                "https://studio.tail1234.ts.net:8443",
                "https://nas.tail1234.ts.net:8443"
            ]
        );
    }

    #[tokio::test]
    async fn shows_tailscales_link_when_serve_waits_for_the_tailnet_to_allow_it() {
        struct Waits(Arc<FakeTailscale>);
        #[async_trait]
        impl RunTailscale for Waits {
            async fn run(&self, args: &[String], timeout: Duration) -> Option<TailscaleResult> {
                if args[1] == "--bg" {
                    return Some(TailscaleResult {
                        code: 1,
                        stdout: "Serve is not enabled on your tailnet.\nTo enable, visit:\n\n https://login.tailscale.com/f/serve?node=abc\n".into(),
                        stderr: String::new(),
                        timed_out: true,
                    });
                }
                self.0.run(args, timeout).await
            }
        }
        let remote = remote(Arc::new(Waits(FakeTailscale::new(serde_json::json!({})))));
        remote.refresh().await;
        remote.set_enabled(true).await;
        assert!(
            problem(remote.status())
                .unwrap()
                .contains("https://login.tailscale.com/f/serve?node=abc")
        );
    }
}
