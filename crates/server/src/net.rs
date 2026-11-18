// What remote.rs, linker.rs and connect/ share: HTTP through an injectable Fetch (the TS injects `fetch` so tests
// never reach Linker or Clerk), lists of change listeners, files only this user reads, and the few JavaScript
// behaviours the file formats and messages depend on.

use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// An HTTP request, as the TS passes to `fetch`.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    /// AbortSignal.timeout in the TS; None waits as long as it takes.
    pub timeout: Option<Duration>,
}

impl Request {
    pub fn new(method: &str, url: impl Into<String>) -> Self {
        Request {
            method: method.to_string(),
            url: url.into(),
            headers: Vec::new(),
            body: None,
            timeout: None,
        }
    }

    pub fn get(url: impl Into<String>) -> Self {
        Request::new("GET", url)
    }

    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_string(), value.into()));
        self
    }

    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// A header's value by name, ignoring case.
    pub fn header_value(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Response {
            status,
            body: body.into(),
        }
    }

    pub fn json(status: u16, body: &Value) -> Self {
        Response::new(status, body.to_string())
    }

    /// Response.ok: a 2xx status.
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The body as JSON, or None when it isn't (`res.json().catch(...)`).
    pub fn json_value(&self) -> Option<Value> {
        serde_json::from_slice(&self.body).ok()
    }
}

/// `fetch`: answers any HTTP status, and fails only when the server can't be reached or took too long.
#[async_trait]
pub trait Fetch: Send + Sync {
    async fn fetch(&self, request: Request) -> anyhow::Result<Response>;
}

/// The real Fetch, with reqwest.
#[derive(Debug, Clone, Default)]
pub struct HttpFetch {
    client: reqwest::Client,
}

#[async_trait]
impl Fetch for HttpFetch {
    async fn fetch(&self, request: Request) -> anyhow::Result<Response> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())?;
        let mut builder = self.client.request(method, &request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = request.body {
            builder = builder.body(body);
        }
        if let Some(timeout) = request.timeout {
            builder = builder.timeout(timeout);
        }
        let response = builder.send().await?;
        let status = response.status().as_u16();
        let body = response.bytes().await?.to_vec();
        Ok(Response { status, body })
    }
}

/// Called when something changed; read the new status from the service.
pub type Listener = Arc<dyn Fn() + Send + Sync>;

/// The TS's `new Set<() => void>()` of listeners. Called without any lock held, so a listener may read the status.
#[derive(Default)]
pub struct Listeners(Mutex<Vec<Listener>>);

impl Listeners {
    pub fn add(&self, listener: Listener) {
        self.0.lock().unwrap().push(listener);
    }

    pub fn notify(&self) {
        let listeners = self.0.lock().unwrap().clone();
        for listener in listeners {
            listener();
        }
    }
}

/// Milliseconds since 1970, as Date.now(); injected in tests.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

pub fn system_clock() -> Clock {
    Arc::new(|| chrono::Utc::now().timestamp_millis())
}

/// new Date(ms).toISOString(): 2026-11-16T10:00:00.000Z.
pub fn iso_date(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Writes a file with the folders it needs, readable only by this user (`{ mode: 0o600 }`, which, as in Node, only
/// applies when the file is made).
pub fn write_private(file: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(file)?;
        out.write_all(text.as_bytes())
    }
    #[cfg(not(unix))]
    std::fs::write(file, text)
}

/// JSON.stringify(value, null, 2).
pub fn pretty_json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).expect("plain data serializes")
}

/// Random bytes from the system.
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("the system has randomness");
    bytes
}

/// Buffer.toString("base64url").
pub fn base64url(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn sha256(text: &str) -> Vec<u8> {
    Sha256::digest(text.as_bytes()).to_vec()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// encodeURIComponent.
pub fn encode_uri_component(text: &str) -> String {
    const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'!')
        .remove(b'~')
        .remove(b'*')
        .remove(b'\'')
        .remove(b'(')
        .remove(b')');
    utf8_percent_encode(text, COMPONENT).to_string()
}

/// text.slice(0, n): JavaScript counts UTF-16 units. A character cut in half becomes U+FFFD.
pub fn slice_utf16(text: &str, n: usize) -> String {
    let units: Vec<u16> = text.encode_utf16().take(n).collect();
    String::from_utf16_lossy(&units)
}

/// A string field that isn't empty, as the TS's `typeof value === "string" && value`.
pub fn text_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// Milliseconds since 1970 written by either app: whole numbers, or JavaScript numbers with a fraction.
pub fn millis<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
    let value = f64::deserialize(deserializer)?;
    Ok(value as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_like_javascript() {
        assert_eq!(encode_uri_component("a b/c?d=é"), "a%20b%2Fc%3Fd%3D%C3%A9");
        assert_eq!(encode_uri_component("-_.!~*'()"), "-_.!~*'()");
        assert_eq!(slice_utf16("abc", 2), "ab");
        assert_eq!(iso_date(0), "1970-01-01T00:00:00.000Z");
    }
}
