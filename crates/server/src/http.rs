// HTTP: /ws for the UI, /api for the agents' tools and the computer menu, files for the players, and the built
// Svelte UI. The port of server/http.ts, on axum.
//
// Every request first passes the gate (remote::request_source): this computer itself, the owner's devices through
// Tailscale Serve while remote access is on, or Linker Connect's tunnel with a session. Anything else gets 403.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use linker_edit_protocol::{
    ClientMessage, Computer, ComputerVia, ConnectDeviceKind, LinkedComputer,
};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::app::{App, Client};
use crate::assets::web_file;
use crate::connect::connect::{Connect, Redeemed, Refusal};
use crate::connect::sessions::{SESSION_COOKIE, Sessions, bearer_token};
use crate::js;
use crate::linker::Linker;
use crate::remote::{
    LOOPBACK_HOSTS, LinkerEditProbe, Probe, RemoteAccessService, RemoteGate, RequestSource,
    request_source,
};

fn mime(file: &str) -> &'static str {
    match js::extname(file) {
        ".html" => "text/html",
        ".js" => "text/javascript",
        ".css" => "text/css",
        ".svg" => "image/svg+xml",
        ".png" => "image/png",
        ".json" => "application/json",
        ".woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// `new URL(text)`'s host: its hostname, with the port when it isn't the scheme's default.
fn url_host(url: &Url) -> Option<String> {
    let hostname = url.host_str()?;
    Some(match url.port() {
        Some(port) => format!("{hostname}:{port}"),
        None => hostname.to_string(),
    })
}

/// Browsers let any website open a WebSocket to localhost, and this socket drives an agent that runs
/// commands. Only this app's own pages may connect: same origin as the request, which also turns away
/// other apps served on localhost. Non-browser clients send no Origin.
pub fn is_allowed_origin(origin: Option<&str>, host: Option<&str>) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    let Ok(url) = Url::parse(origin) else {
        return false;
    };
    (url.scheme() == "http" || url.scheme() == "https")
        && url
            .host_str()
            .is_some_and(|name| LOOPBACK_HOSTS.contains(&name))
        && url_host(&url)
            .is_some_and(|own| Some(own.as_str()) == host.map(str::to_lowercase).as_deref())
}

/// The built file for a URL path, index.html for client-side routes, or None when the UI isn't built.
pub fn static_file(dist_dir: &Path, url_path: &str) -> Option<PathBuf> {
    let dist = js::normalize(dist_dir);
    let file = js::normalize(Path::new(&format!(
        "{}{}{url_path}",
        dist.display(),
        std::path::MAIN_SEPARATOR
    )));
    let inside = file
        .strip_prefix(&dist)
        .ok()
        .is_some_and(|relative| !relative.as_os_str().is_empty());
    if inside && file.is_file() {
        return Some(file);
    }
    let index = dist.join("index.html");
    index.is_file().then_some(index)
}

/// What a Range header asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteRange {
    /// No range, or one this server doesn't do (several parts): the whole file.
    Whole,
    Part {
        start: u64,
        end: u64,
    },
    /// A range past the end.
    Invalid,
}

/// The bytes a Range header asks for of a file of size bytes.
pub fn byte_range(header: Option<&str>, size: u64) -> ByteRange {
    let Some(spec) = header.and_then(|header| header.strip_prefix("bytes=")) else {
        return ByteRange::Whole;
    };
    let Some((first, last)) = spec.split_once('-') else {
        return ByteRange::Whole;
    };
    let digits = |text: &str| text.chars().all(|c| c.is_ascii_digit());
    if !digits(first) || !digits(last) || (first.is_empty() && last.is_empty()) {
        return ByteRange::Whole;
    }
    let number = |text: &str| text.parse::<u128>().unwrap_or(u128::MAX);
    let size = u128::from(size);
    let (start, end): (i128, i128) = if first.is_empty() {
        let suffix = number(last);
        (size.saturating_sub(suffix) as i128, size as i128 - 1)
    } else {
        let end = if last.is_empty() {
            size as i128 - 1
        } else {
            number(last).min(size.saturating_sub(1)) as i128
        };
        (number(first).min(i128::MAX as u128) as i128, end)
    };
    if start > end || start >= size as i128 {
        return ByteRange::Invalid;
    }
    ByteRange::Part {
        start: start as u64,
        end: end as u64,
    }
}

const ESCAPES: [(char, &str); 5] = [
    ('&', "&amp;"),
    ('<', "&lt;"),
    ('>', "&gt;"),
    ('"', "&quot;"),
    ('\'', "&#39;"),
];

fn escape_html(text: &str) -> String {
    text.chars()
        .map(|c| match ESCAPES.iter().find(|(from, _)| *from == c) {
            Some((_, to)) => (*to).to_string(),
            None => c.to_string(),
        })
        .collect()
}

/// A small page in the browser tab that signed in, which Linker-Edit itself doesn't show.
pub fn linker_page(title: &str, text: &str) -> String {
    let (title, text) = (escape_html(title), escape_html(text));
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} · Linker-Edit</title><link rel="icon" href="/favicon.svg" type="image/svg+xml">
<style>body{{margin:0;min-height:100vh;display:grid;place-items:center;font:15px/1.5 system-ui,sans-serif;background:#0b0b0c;color:#e8e8ea}}
main{{max-width:26rem;padding:2rem}}h1{{font-size:1.25rem;margin:0 0 .5rem}}p{{margin:0;color:#a1a1aa}}
@media (prefers-color-scheme: light){{body{{background:#fafafa;color:#18181b}}p{{color:#52525b}}}}</style></head>
<body><main><h1>{title}</h1><p>{text}</p></main></body></html>"#
    )
}

/// decodeURIComponent: None for a % that isn't followed by two hex digits, or bytes that aren't UTF-8.
fn decode_uri_component(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = text.get(index + 1..index + 3)?;
            if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hostname() -> String {
    gethostname::gethostname().to_string_lossy().into_owned()
}

/// Lets the user's own devices in through Tailscale Serve while remote access is on.
#[async_trait]
pub trait RemoteRoutes: Send + Sync {
    fn gate(&self) -> Option<RemoteGate>;
    async fn computers(&self) -> Vec<Computer>;
}

#[async_trait]
impl RemoteRoutes for RemoteAccessService {
    fn gate(&self) -> Option<RemoteGate> {
        RemoteAccessService::gate(self)
    }
    async fn computers(&self) -> Vec<Computer> {
        RemoteAccessService::computers(self).await
    }
}

/// No remote access, as in tests.
pub struct NoRemote;

#[async_trait]
impl RemoteRoutes for NoRemote {
    fn gate(&self) -> Option<RemoteGate> {
        None
    }
    async fn computers(&self) -> Vec<Computer> {
        Vec::new()
    }
}

/// Signing in with Linker in the browser; see linker.rs.
#[async_trait]
pub trait LinkerRoutes: Send + Sync {
    /// Where to send the browser to sign in.
    async fn start(&self) -> anyhow::Result<String>;
    /// The callback's query: the name signed in.
    async fn finish(&self, query: &HashMap<String, String>) -> anyhow::Result<String>;
}

#[async_trait]
impl LinkerRoutes for Linker {
    async fn start(&self) -> anyhow::Result<String> {
        Linker::start(self).await
    }
    async fn finish(&self, query: &HashMap<String, String>) -> anyhow::Result<String> {
        Linker::finish(self, query).await
    }
}

/// Linker Connect (connect/connect.rs): this computer's tunnel, and opening your other computers.
#[async_trait]
pub trait ConnectRoutes: Send + Sync {
    /// This computer's address at Linker while the tunnel runs.
    fn host(&self) -> Option<String>;
    /// A one-time code for a device that showed this account's Linker token.
    async fn admit(&self, token: &str, device: &str) -> Result<String, Refusal>;
    fn redeem(&self, code: &str) -> Result<Redeemed, Refusal>;
    async fn open(&self, url: &str) -> anyhow::Result<String>;
    async fn others(&self) -> Vec<LinkedComputer>;
}

#[async_trait]
impl ConnectRoutes for Connect {
    fn host(&self) -> Option<String> {
        Connect::host(self)
    }
    async fn admit(&self, token: &str, device: &str) -> Result<String, Refusal> {
        Connect::admit(self, token, device).await
    }
    fn redeem(&self, code: &str) -> Result<Redeemed, Refusal> {
        Connect::redeem(self, code)
    }
    async fn open(&self, url: &str) -> anyhow::Result<String> {
        Connect::open(self, url).await
    }
    async fn others(&self) -> Vec<LinkedComputer> {
        Connect::others(self).await
    }
}

pub struct ConnectSide {
    pub routes: Arc<dyn ConnectRoutes>,
    pub sessions: Arc<Sessions>,
    /// Asks another computer's Linker-Edit its name and version; tests replace it.
    pub probe: Arc<dyn Probe>,
}

/// Where the Svelte UI comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Web {
    /// The build inside the binary (assets::web_file).
    Embedded,
    /// A build folder on disk, as tests use.
    Dir(PathBuf),
}

pub struct HttpOptions {
    pub app: App,
    pub web: Web,
    /// Lets the agents' MCP server call /api; web pages and other programs can't.
    pub api_token: String,
    pub remote: Arc<dyn RemoteRoutes>,
    pub linker: Option<Arc<dyn LinkerRoutes>>,
    pub connect: Option<ConnectSide>,
}

impl HttpOptions {
    pub fn new(app: App, web: Web, api_token: impl Into<String>) -> Self {
        HttpOptions {
            app,
            web,
            api_token: api_token.into(),
            remote: Arc::new(NoRemote),
            linker: None,
            connect: None,
        }
    }
}

struct Server {
    options: HttpOptions,
    // A device signed out of Linker Connect is cut off at once, not when it next reconnects.
    by_session: Mutex<HashMap<String, Vec<(u64, CancellationToken)>>>,
    next_socket: std::sync::atomic::AtomicU64,
}

/// The whole HTTP side, ready for axum::serve.
pub fn router(options: HttpOptions) -> Router {
    let server = Arc::new(Server {
        options,
        by_session: Mutex::default(),
        next_socket: std::sync::atomic::AtomicU64::new(1),
    });
    if let Some(connect) = &server.options.connect {
        let me: Weak<Server> = Arc::downgrade(&server);
        connect.sessions.on_revoke(Arc::new(move |ids: &[String]| {
            let Some(server) = me.upgrade() else { return };
            let sockets = server.by_session.lock().unwrap();
            for id in ids {
                for (_, closed) in sockets.get(id).into_iter().flatten() {
                    closed.cancel();
                }
            }
        }));
    }
    Router::new().fallback(handle).with_state(server)
}

fn header_text<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn json_response(status: u16, body: &Value) -> Response {
    let mut response = (status_code(status), js::stringify(body)).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn page(status: u16, title: &str, text: &str) -> Response {
    let mut response = (status_code(status), linker_page(title, text)).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}

fn status_code(status: u16) -> StatusCode {
    StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

fn plain(status: u16, text: &'static str) -> Response {
    (status_code(status), text).into_response()
}

fn empty(status: u16) -> Response {
    status_code(status).into_response()
}

fn error_body(error: impl std::fmt::Display) -> Value {
    json!({ "error": error.to_string() })
}

/// A JSON body that is an object; anything else reads as an empty one. Err when it isn't JSON or is larger than limit.
async fn read_json(body: Body, limit: usize) -> anyhow::Result<Map<String, Value>> {
    let bytes = axum::body::to_bytes(body, limit)
        .await
        .map_err(|_| anyhow::anyhow!("Too large."))?;
    let text = std::str::from_utf8(&bytes)?;
    let value: Value = serde_json::from_str(if text.is_empty() { "{}" } else { text })?;
    Ok(match value {
        Value::Object(object) => object,
        _ => Map::new(),
    })
}

/// A small JSON body; anything larger than 4096 bytes is refused.
async fn read_small_json(body: Body) -> anyhow::Result<Map<String, Value>> {
    read_json(body, 4096).await
}

fn object(value: Option<&Value>) -> Map<String, Value> {
    match value {
        Some(Value::Object(object)) => object.clone(),
        _ => Map::new(),
    }
}

impl Server {
    fn connect_host(&self) -> Option<String> {
        self.options
            .connect
            .as_ref()
            .and_then(|connect| connect.routes.host())
    }

    fn source_of(&self, headers: &HeaderMap) -> Option<RequestSource> {
        let gate = self.options.remote.gate();
        request_source(headers, gate.as_ref(), self.connect_host().as_deref())
    }

    /// The Linker Connect session of a request through the tunnel: the iPad app's Bearer token, or else
    /// Linker-Edit's cookie. Called only for "connect" requests, so a Bearer token never changes how others get in.
    fn session_of(&self, headers: &HeaderMap) -> Option<String> {
        let connect = self.options.connect.as_ref()?;
        connect.sessions.session_of(
            header_text(headers, "authorization"),
            header_text(headers, "cookie"),
        )
    }

    /// POST routes for processes started for the agents, such as their MCP servers. Any web page can POST to
    /// localhost, hence the token.
    async fn agent_call<F, Fut>(
        &self,
        method: &Method,
        headers: &HeaderMap,
        body: Body,
        run: F,
    ) -> Response
    where
        F: FnOnce(Map<String, Value>) -> Fut,
        Fut: Future<Output = anyhow::Result<Value>>,
    {
        let respond = |status: StatusCode, body: &Value| {
            let mut response = (status, js::stringify(body)).into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
            response
        };
        let expected = format!("Bearer {}", self.options.api_token);
        if method != Method::POST
            || header_text(headers, "authorization") != Some(expected.as_str())
        {
            return respond(StatusCode::FORBIDDEN, &json!({ "error": "Not allowed." }));
        }
        let result = match read_json(body, usize::MAX).await {
            Ok(body) => run(body).await,
            Err(error) => Err(error),
        };
        match result {
            Ok(value) => respond(StatusCode::OK, &value),
            Err(error) => respond(StatusCode::BAD_REQUEST, &error_body(error)),
        }
    }

    fn web_response(&self, url_path: &str) -> Response {
        let found: Option<(Cow<'static, [u8]>, &'static str)> = match &self.options.web {
            Web::Dir(dist) => static_file(dist, url_path).and_then(|file| {
                let data = std::fs::read(&file).ok()?;
                Some((Cow::Owned(data), mime(&file.to_string_lossy())))
            }),
            Web::Embedded => {
                let path = url_path.trim_start_matches('/');
                web_file(path)
                    .map(|data| (data, mime(path)))
                    .or_else(|| web_file("index.html").map(|data| (data, "text/html")))
            }
        };
        let Some((data, content_type)) = found else {
            return page(
                404,
                "Linker-Edit's UI isn't built",
                "Run npm run build in apps/desktop, then reload this page. npm run dev serves it while you work on it.",
            );
        };
        let mut response = (StatusCode::OK, Body::from(data.into_owned())).into_response();
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
        response
    }
}

// Video players seek with Range requests, so files go out in parts when asked.
async fn send_file(
    method: &Method,
    headers: &HeaderMap,
    file: &Path,
    content_type: &str,
) -> Response {
    let Ok(mut opened) = tokio::fs::File::open(file).await else {
        return empty(404);
    };
    let Ok(size) = opened.metadata().await.map(|meta| meta.len()) else {
        return empty(404);
    };
    let range = byte_range(header_text(headers, "range"), size);
    let (start, end) = match range {
        ByteRange::Invalid => {
            let mut response = empty(416);
            if let Ok(value) = HeaderValue::from_str(&format!("bytes */{size}")) {
                response.headers_mut().insert(header::CONTENT_RANGE, value);
            }
            return response;
        }
        ByteRange::Part { start, end } => (start, end),
        ByteRange::Whole => (0, size.saturating_sub(1)),
    };
    let length = if size == 0 { 0 } else { end - start + 1 };
    let body = if *method == Method::HEAD || length == 0 {
        Body::empty()
    } else if opened.seek(std::io::SeekFrom::Start(start)).await.is_err() {
        return empty(500);
    } else {
        Body::from_stream(tokio_util::io::ReaderStream::new(opened.take(length)))
    };
    let status = if matches!(range, ByteRange::Part { .. }) {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    let mut response = (status, body).into_response();
    let out = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(content_type) {
        out.insert(header::CONTENT_TYPE, value);
    }
    out.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    out.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    out.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=300"),
    );
    if let ByteRange::Part { start, end } = range
        && let Ok(value) = HeaderValue::from_str(&format!("bytes {start}-{end}/{size}"))
    {
        out.insert(header::CONTENT_RANGE, value);
    }
    response
}

async fn handle(State(server): State<Arc<Server>>, request: Request) -> Response {
    let raw = request
        .uri()
        .path_and_query()
        .map_or("/", |path| path.as_str())
        .to_string();
    let raw_path = raw.split('?').next().unwrap_or("/");
    let upgrade = header_text(request.headers(), "upgrade")
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
    if raw_path == "/ws" && upgrade {
        return websocket(server, request).await;
    }
    let (parts, body) = request.into_parts();
    let (method, headers) = (&parts.method, &parts.headers);
    // Also turns away pages on other sites whose name was pointed at 127.0.0.1 (DNS rebinding).
    let Some(source) = server.source_of(headers) else {
        return plain(
            403,
            "Not allowed. Remote access is only for devices logged in to this computer's Tailscale account.",
        );
    };
    let parsed = Url::parse(&format!("http://localhost{raw}"))
        .ok()
        .and_then(|url| Some((decode_uri_component(url.path())?, url)));
    let Some((url_path, url)) = parsed else {
        return plain(400, "Bad request");
    };
    let mut query: HashMap<String, String> = HashMap::new();
    for (key, value) in url.query_pairs() {
        query
            .entry(key.into_owned())
            .or_insert_with(|| value.into_owned());
    }
    let app = &server.options.app;
    // How the desktop app on the user's other computers recognizes this one.
    if url_path == "/api/hello" {
        return json_response(
            200,
            &json!({ "app": "linker-edit", "name": hostname(), "version": app.version() }),
        );
    }
    if source == RequestSource::Connect
        && let Some(connect) = &server.options.connect
    {
        // Through the Linker Connect tunnel, only these three get in without a session: they make one.
        if url_path == "/api/connect/session" {
            let auth = header_text(headers, "authorization")
                .and_then(|value| value.strip_prefix("Bearer "))
                .filter(|token| !token.is_empty())
                .map(str::to_string);
            let Some(auth) = auth.filter(|_| method == Method::POST) else {
                return json_response(
                    401,
                    &json!({ "error": "Open this computer from Linker-Edit's computer menu, signed in with Linker." }),
                );
            };
            let Ok(body) = read_small_json(body).await else {
                return json_response(400, &json!({ "error": "Bad request." }));
            };
            let name = body.get("name").and_then(Value::as_str).unwrap_or("");
            return match connect.routes.admit(&auth, name).await {
                Ok(code) => json_response(200, &json!({ "code": code })),
                Err(refusal) => json_response(refusal.status, &json!({ "error": refusal.error })),
            };
        }
        if url_path == "/api/connect/enter" {
            let code = query.get("code").map_or("", String::as_str);
            let Some(entered) = connect.sessions.enter(code, ConnectDeviceKind::Cookie) else {
                return page(
                    401,
                    "That link ran out",
                    &format!(
                        "Open {} again from Linker-Edit's computer menu.",
                        hostname()
                    ),
                );
            };
            let mut response = empty(302);
            let out = response.headers_mut();
            out.insert(header::LOCATION, HeaderValue::from_static("/"));
            out.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            out.insert(
                header::REFERRER_POLICY,
                HeaderValue::from_static("no-referrer"),
            );
            if let Ok(cookie) = HeaderValue::from_str(&format!(
                "{SESSION_COOKIE}={}; Path=/; Max-Age={}; HttpOnly; Secure; SameSite=Lax",
                entered.token, entered.max_age_seconds
            )) {
                out.insert(header::SET_COOKIE, cookie);
            }
            return response;
        }
        // The iPad app swaps its code for a session it sends as a Bearer token: in the body, never in the address.
        if url_path == "/api/connect/native" {
            if method != Method::POST {
                return json_response(405, &json!({ "error": "POST the code." }));
            }
            let Ok(body) = read_small_json(body).await else {
                return json_response(400, &json!({ "error": "Bad request." }));
            };
            let code = body.get("code").and_then(Value::as_str).unwrap_or("");
            return match connect.routes.redeem(code) {
                Ok(redeemed) => {
                    json_response(200, &serde_json::to_value(redeemed).unwrap_or_default())
                }
                Err(refusal) => json_response(refusal.status, &json!({ "error": refusal.error })),
            };
        }
        if server.session_of(headers).is_none() {
            if bearer_token(header_text(headers, "authorization")).is_some() {
                // The iPad app signs in again when it gets this.
                let mut response = json_response(
                    401,
                    &json!({ "error": "Signed out. Sign in with Linker again." }),
                );
                response.headers_mut().insert(
                    header::WWW_AUTHENTICATE,
                    HeaderValue::from_static("Bearer error=\"invalid_token\""),
                );
                return response;
            }
            return page(
                401,
                "Signed out",
                &format!(
                    "Open {} from Linker-Edit's computer menu, signed in with the same Linker account.",
                    hostname()
                ),
            );
        }
    }
    // The desktop app opening another of your Linker Connect computers: only from this computer itself.
    if url_path == "/api/connect/open" {
        let allowed = source == RequestSource::Local
            && method == Method::POST
            && is_allowed_origin(header_text(headers, "origin"), header_text(headers, "host"));
        let Some(connect) = server.options.connect.as_ref().filter(|_| allowed) else {
            return json_response(403, &json!({ "error": "Not allowed." }));
        };
        let opened = async {
            let body = read_small_json(body).await?;
            let Some(url) = body.get("url").and_then(Value::as_str) else {
                anyhow::bail!("url must be the computer's address.");
            };
            connect.routes.open(url).await
        }
        .await;
        return match opened {
            Ok(url) => json_response(200, &json!({ "url": url })),
            Err(error) => json_response(400, &error_body(error)),
        };
    }
    if url_path == "/api/computers" {
        // Your other computers linked with Linker Connect, each asked its name and version through its tunnel.
        let linked = async {
            let Some(connect) = &server.options.connect else {
                return Vec::new();
            };
            let others = connect.routes.others().await;
            let probes = others.into_iter().map(|computer| {
                let probe = connect.probe.clone();
                async move {
                    let origin = Url::parse(&computer.url)
                        .map(|url| url.origin().ascii_serialization())
                        .unwrap_or_else(|_| computer.url.clone());
                    let hello = if computer.online == Some(false) {
                        None
                    } else {
                        probe.probe(&origin).await
                    };
                    Computer {
                        name: hello
                            .as_ref()
                            .map(|hello| hello.name.clone())
                            .filter(|name| !name.is_empty())
                            .unwrap_or(computer.name),
                        url: origin,
                        version: hello
                            .as_ref()
                            .map(|hello| hello.version.clone())
                            .unwrap_or_default(),
                        via: Some(ComputerVia::Linker),
                        online: Some(hello.is_some()),
                    }
                }
            });
            futures::future::join_all(probes).await
        };
        let (mut others, linked) = futures::join!(server.options.remote.computers(), linked);
        others.extend(linked);
        return json_response(
            200,
            &json!({ "name": hostname(), "version": app.version(), "others": others }),
        );
    }
    if url_path == "/api/linker/sign-in" || url_path == "/api/linker/callback" {
        let Some(linker) = &server.options.linker else {
            return page(
                404,
                "Linker sign-in is off",
                "This Linker-Edit can't sign in to Linker.",
            );
        };
        // Linker sends the browser back to 127.0.0.1, which is only this computer.
        if source != RequestSource::Local {
            return page(
                403,
                "Sign in on the computer itself",
                "Linker can only sign in on the computer Linker-Edit runs on.",
            );
        }
        if url_path == "/api/linker/sign-in" {
            return match linker.start().await {
                Ok(location) => {
                    let mut response = empty(302);
                    let out = response.headers_mut();
                    if let Ok(location) = HeaderValue::from_str(&location) {
                        out.insert(header::LOCATION, location);
                    }
                    out.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                    out.insert(
                        header::REFERRER_POLICY,
                        HeaderValue::from_static("no-referrer"),
                    );
                    response
                }
                Err(error) => page(502, "Couldn't reach Linker", &error.to_string()),
            };
        }
        return match linker.finish(&query).await {
            Ok(name) => page(
                200,
                &format!("Signed in as {name}"),
                "You're signed in to Linker. Close this tab and go back to Linker-Edit.",
            ),
            Err(error) => page(400, "Not signed in", &error.to_string()),
        };
    }
    if url_path == "/api/transcribe" {
        return server
            .agent_call(method, headers, body, |body| async move {
                let files: Option<Vec<String>> = match body.get("files") {
                    Some(Value::Array(files)) => files
                        .iter()
                        .map(|file| file.as_str().map(str::to_string))
                        .collect(),
                    _ => None,
                };
                let Some(files) = files else {
                    anyhow::bail!("files must be a list of file paths.");
                };
                let language = body.get("language").and_then(Value::as_str);
                Ok(json!({ "text": app.transcribe(&files, language).await? }))
            })
            .await;
    }
    let tool_route = matches!(
        url_path.as_str(),
        "/api/resolve-tool" | "/api/library-tool" | "/api/study-tool" | "/api/brief-tool"
    );
    if tool_route {
        let route = url_path.clone();
        return server
            .agent_call(method, headers, body, |body| async move {
                let Some(tool) = body.get("tool").and_then(Value::as_str) else {
                    anyhow::bail!("tool must be a tool name.");
                };
                let input = object(body.get("input"));
                let reply = match route.as_str() {
                    "/api/resolve-tool" => app.resolve_tool(tool, Value::Object(input)).await,
                    "/api/library-tool" => app.library_tool(tool, input).await,
                    "/api/study-tool" => app.study_tool(tool, input).await,
                    _ => app.brief_tool(tool, input).await,
                };
                Ok(serde_json::to_value(reply)?)
            })
            .await;
    }
    // Plays library files on the library page. Every request already passed the gate above: this computer, or
    // the owner's own devices over Tailscale. Only files inside library folders go out; see library/serve.rs.
    if let Some(rest) = url_path.strip_prefix("/library/") {
        let (folder_id, rel) = rest.split_once('/').unwrap_or((rest, ""));
        let file = (method == Method::GET || method == Method::HEAD)
            .then(|| app.library_file(folder_id, rel))
            .flatten();
        let Some(file) = file else {
            let mut response = empty(404);
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            return response;
        };
        // A file is only ever shown as media, never run as a page.
        let mut response = send_file(method, headers, &file.path, file.content_type).await;
        let out = response.headers_mut();
        out.insert(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
        out.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'none'; sandbox"),
        );
        return response;
    }
    if url_path == "/api/timeline" {
        let get = |key: &str| query.get(key).filter(|value| !value.is_empty()).cloned();
        let project = get("project");
        return match app
            .playable_timeline(project.as_deref(), get("id"), get("commit"))
            .await
        {
            Ok(Some(timeline)) => {
                json_response(200, &serde_json::to_value(timeline).unwrap_or_default())
            }
            Ok(None) => json_response(
                404,
                &json!({ "error": "That timeline isn't in Resolve or in that save." }),
            ),
            Err(error) => json_response(500, &error_body(error)),
        };
    }
    if let Some(id) = url_path.strip_prefix("/media/") {
        return match app.media_file(id) {
            Some(media) => send_file(method, headers, Path::new(&media.path), media.r#type).await,
            None => empty(404),
        };
    }
    if let Some(name) = url_path.strip_prefix("/preview/") {
        return match app.preview_file(name) {
            Some(file) => send_file(method, headers, &file, "video/mp4").await,
            None => empty(404),
        };
    }
    server.web_response(&url_path)
}

/// Sends what the app says to one socket, through a channel its task empties.
struct SocketClient(mpsc::UnboundedSender<String>);

impl Client for SocketClient {
    fn send(&self, data: String) {
        let _ = self.0.send(data);
    }
}

async fn websocket(server: Arc<Server>, request: Request) -> Response {
    let (mut parts, _) = request.into_parts();
    let headers = parts.headers.clone();
    let origin = header_text(&headers, "origin");
    let source = server.source_of(&headers);
    let allowed = match source {
        Some(RequestSource::Local) => is_allowed_origin(origin, header_text(&headers, "host")),
        Some(RequestSource::Remote) => server
            .options
            .remote
            .gate()
            .is_some_and(|gate| origin == Some(format!("https://{}", gate.host).as_str())),
        Some(RequestSource::Connect) => {
            // A page can't add an Authorization header to a WebSocket, so a Bearer token comes from a native app,
            // which sends no Origin, or one naming this address. A cookie comes from a page: only this computer's own.
            let own = server
                .connect_host()
                .is_some_and(|host| origin == Some(format!("https://{host}").as_str()));
            let allowed = if bearer_token(header_text(&headers, "authorization")).is_none() {
                own
            } else {
                own || origin.is_none()
            };
            allowed && server.session_of(&headers).is_some()
        }
        None => false,
    };
    if !allowed {
        return empty(401);
    }
    let upgrade = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
        Ok(upgrade) => upgrade,
        Err(rejection) => return rejection.into_response(),
    };
    let session = (source == Some(RequestSource::Connect))
        .then(|| server.session_of(&headers))
        .flatten();
    upgrade.on_upgrade(move |socket| serve_socket(server, socket, session))
}

async fn serve_socket(server: Arc<Server>, socket: WebSocket, session: Option<String>) {
    let closed = CancellationToken::new();
    let socket_id = server
        .next_socket
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if let Some(session) = &session {
        server
            .by_session
            .lock()
            .unwrap()
            .entry(session.clone())
            .or_default()
            .push((socket_id, closed.clone()));
    }
    let app = server.options.app.clone();
    let (sender, mut outgoing) = mpsc::unbounded_channel();
    let client = app.connect(Arc::new(SocketClient(sender)));
    let (mut sink, mut stream) = socket.split();
    loop {
        tokio::select! {
            Some(text) = outgoing.recv() => {
                if sink.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
            incoming = stream.next() => {
                let text = match incoming {
                    Some(Ok(Message::Text(text))) => text.to_string(),
                    Some(Ok(Message::Binary(data))) => String::from_utf8_lossy(&data).into_owned(),
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                    Some(Ok(_)) => continue,
                };
                // A message that isn't one of the protocol's is dropped: the UI never sends one.
                if let Ok(message) = serde_json::from_str::<ClientMessage>(&text) {
                    app.handle_message(client, message);
                }
            }
            () = closed.cancelled() => {
                let _ = sink.send(Message::Close(None)).await;
                break;
            }
        }
    }
    app.disconnect(client);
    if let Some(session) = session {
        let mut sockets = server.by_session.lock().unwrap();
        if let Some(list) = sockets.get_mut(&session) {
            list.retain(|(id, _)| *id != socket_id);
            if list.is_empty() {
                sockets.remove(&session);
            }
        }
    }
}

/// The real probe for /api/computers when Connect is on.
pub fn default_probe() -> Arc<dyn Probe> {
    Arc::new(LinkerEditProbe)
}

#[cfg(test)]
#[path = "http/tests.rs"]
mod tests;

// The iPad app's fixtures, recorded from a real session over /ws with RECORD_IPAD_FIXTURES=1.
#[cfg(test)]
#[path = "http/ipad_fixtures.rs"]
mod ipad_fixtures;
