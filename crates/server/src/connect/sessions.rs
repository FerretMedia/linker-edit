// The devices that opened this computer through Linker Connect. Getting in has two steps, so the session itself
// never travels in an address:
// 1. Your other Linker-Edit, or the iPad app, proves it's your Linker account and gets a one-time code (60 seconds).
// 2. Linker-Edit's window opens /api/connect/enter?code=…, which swaps the code for a session cookie; the iPad app
//    posts the code to /api/connect/native for a session it sends as a Bearer token.
// A session only works the way it was given out: a cookie's is never taken as a Bearer token, or the other way.
// Only hashes of codes and sessions are kept, in memory and in the file.
//
// The http layer's checks on requests through the tunnel are here too, on plain header values: bearer_token,
// cookie_value and Sessions::session_of.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use linker_edit_protocol::{ConnectDevice, ConnectDeviceKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::net::{
    Clock, Listener, Listeners, base64url, hex, millis, pretty_json, random_bytes, sha256,
    slice_utf16, system_clock, write_private,
};

const CODE_LIFETIME: i64 = 60_000;
pub const SESSION_LIFETIME: i64 = 30 * 24 * 60 * 60_000;
/// last_seen_at is written at most this often.
const TOUCH_EVERY: i64 = 60_000;

/// The cookie Linker-Edit's window keeps its session in.
pub const SESSION_COOKIE: &str = "linker_edit_session";

/// An entry of ~/LinkerEdit/connect-sessions.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    id: String,
    hash: String,
    name: String,
    #[serde(deserialize_with = "millis")]
    created_at: i64,
    #[serde(deserialize_with = "millis")]
    last_seen_at: i64,
    #[serde(deserialize_with = "millis")]
    expires_at: i64,
    /// Missing in sessions from before the iPad app, which were all cookies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<ConnectDeviceKind>,
}

fn hash(secret: &str) -> String {
    hex(&sha256(secret))
}

fn secret() -> String {
    base64url(&random_bytes::<32>())
}

/// Compares two hashes in a time that doesn't tell how much of them matched.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |diff, (x, y)| diff | (x ^ y))
            == 0
}

/// A session given out for a code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entered {
    pub token: String,
    pub max_age_seconds: i64,
    /// Milliseconds since 1970.
    pub expires_at: i64,
}

struct Code {
    name: String,
    expires_at: i64,
}

struct State {
    codes: HashMap<String, Code>,
    sessions: Vec<Saved>,
}

pub type OnRevoke = Arc<dyn Fn(&[String]) + Send + Sync>;

pub struct Sessions {
    file: PathBuf,
    now: Clock,
    state: Mutex<State>,
    revoke_listeners: Mutex<Vec<OnRevoke>>,
    change_listeners: Listeners,
}

impl Sessions {
    /// Sessions kept in file (AppConfig::connect.sessions_file).
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Sessions::with_clock(file, system_clock())
    }

    pub fn with_clock(file: impl Into<PathBuf>, now: Clock) -> Self {
        let file = file.into();
        let at = now();
        let sessions = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str::<Vec<Value>>(&text).ok())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|entry| serde_json::from_value::<Saved>(entry).ok())
            .filter(|entry| entry.expires_at > at)
            .collect();
        Sessions {
            file,
            now,
            state: Mutex::new(State {
                codes: HashMap::new(),
                sessions,
            }),
            revoke_listeners: Mutex::new(Vec::new()),
            change_listeners: Listeners::default(),
        }
    }

    fn now(&self) -> i64 {
        (self.now)()
    }

    fn write(&self, sessions: &[Saved]) {
        if let Err(error) = write_private(&self.file, &pretty_json(&sessions)) {
            tracing::error!("Couldn't keep the Linker Connect sessions: {error}");
        }
    }

    fn drop_sessions(&self, ids: Vec<String>) {
        if ids.is_empty() {
            return;
        }
        let sessions = {
            let mut state = self.state.lock().unwrap();
            state.sessions.retain(|entry| !ids.contains(&entry.id));
            state.sessions.clone()
        };
        self.write(&sessions);
        let listeners = self.revoke_listeners.lock().unwrap().clone();
        for listener in listeners {
            listener(&ids);
        }
        self.change_listeners.notify();
    }

    /// Step 1: a code for the device called name, good once within a minute.
    pub fn issue_code(&self, name: &str) -> String {
        let now = self.now();
        let code = secret();
        let name = slice_utf16(name, 64);
        let mut state = self.state.lock().unwrap();
        state.codes.retain(|_, code| code.expires_at > now);
        state.codes.insert(
            hash(&code),
            Code {
                name: if name.is_empty() {
                    "Another computer".into()
                } else {
                    name
                },
                expires_at: now + CODE_LIFETIME,
            },
        );
        code
    }

    /// Step 2: swaps a code for a session of that kind; None when the code is wrong, used or too old.
    pub fn enter(&self, code: &str, kind: ConnectDeviceKind) -> Option<Entered> {
        let key = hash(code);
        let now = self.now();
        let (entered, sessions) = {
            let mut state = self.state.lock().unwrap();
            let found = state
                .codes
                .keys()
                .find(|entry| same(entry, &key))
                .cloned()?;
            let pending = state.codes.remove(&found)?;
            if pending.expires_at <= now {
                return None;
            }
            let token = secret();
            let expires_at = now + SESSION_LIFETIME;
            state.sessions.push(Saved {
                id: uuid::Uuid::new_v4().to_string(),
                hash: hash(&token),
                name: pending.name,
                created_at: now,
                last_seen_at: now,
                expires_at,
                kind: Some(kind),
            });
            let entered = Entered {
                token,
                max_age_seconds: SESSION_LIFETIME / 1000,
                expires_at,
            };
            (entered, state.sessions.clone())
        };
        self.write(&sessions);
        self.change_listeners.notify();
        Some(entered)
    }

    /// The session id a token belongs to, or None; only a session given out as kind counts.
    pub fn check(&self, token: Option<&str>, kind: ConnectDeviceKind) -> Option<String> {
        let token = token.filter(|token| !token.is_empty())?;
        let key = hash(token);
        let now = self.now();
        let mut state = self.state.lock().unwrap();
        let session = state
            .sessions
            .iter_mut()
            .find(|entry| same(&entry.hash, &key))?;
        if session.kind.unwrap_or(ConnectDeviceKind::Cookie) != kind {
            return None;
        }
        if session.expires_at <= now {
            let id = session.id.clone();
            drop(state);
            self.drop_sessions(vec![id]);
            return None;
        }
        let id = session.id.clone();
        if now - session.last_seen_at > TOUCH_EVERY {
            session.last_seen_at = now;
            let sessions = state.sessions.clone();
            drop(state);
            self.write(&sessions);
        }
        Some(id)
    }

    pub fn list(&self) -> Vec<ConnectDevice> {
        let now = self.now();
        let state = self.state.lock().unwrap();
        state
            .sessions
            .iter()
            .filter(|entry| entry.expires_at > now)
            .map(|entry| ConnectDevice {
                id: entry.id.clone(),
                name: entry.name.clone(),
                kind: entry.kind.unwrap_or(ConnectDeviceKind::Cookie),
                created_at: crate::js::iso_from_ms(entry.created_at),
                last_seen_at: crate::js::iso_from_ms(entry.last_seen_at),
            })
            .collect()
    }

    pub fn revoke(&self, id: &str) {
        let known = self
            .state
            .lock()
            .unwrap()
            .sessions
            .iter()
            .any(|entry| entry.id == id);
        if known {
            self.drop_sessions(vec![id.to_string()]);
        }
    }

    /// Ends every session, and forgets codes not used yet.
    pub fn revoke_all(&self) {
        let ids = {
            let mut state = self.state.lock().unwrap();
            state.codes.clear();
            state
                .sessions
                .iter()
                .map(|entry| entry.id.clone())
                .collect()
        };
        self.drop_sessions(ids);
    }

    /// Called with the ids of sessions that just ended, so their open connections (/ws) close.
    pub fn on_revoke(&self, listener: OnRevoke) {
        self.revoke_listeners.lock().unwrap().push(listener);
    }

    pub fn on_change(&self, listener: Listener) {
        self.change_listeners.add(listener);
    }

    /// The Linker Connect session of a request through the tunnel: the iPad app's Bearer token, or else Linker-Edit's
    /// cookie. Call it only for RequestSource::Connect, so a Bearer token never changes how others get in. A Bearer
    /// header that isn't a session doesn't fall back to a cookie sent with it.
    pub fn session_of(&self, authorization: Option<&str>, cookie: Option<&str>) -> Option<String> {
        match bearer_token(authorization) {
            Some(bearer) => self.check(Some(bearer), ConnectDeviceKind::Native),
            None => self.check(
                cookie_value(cookie, SESSION_COOKIE).as_deref(),
                ConnectDeviceKind::Cookie,
            ),
        }
    }
}

/// The token of an Authorization: Bearer header, or None.
pub fn bearer_token(authorization: Option<&str>) -> Option<&str> {
    let token = authorization?.strip_prefix("Bearer ")?;
    (!token.is_empty() && !token.chars().any(char::is_whitespace)).then_some(token)
}

/// One cookie's value from a Cookie header.
pub fn cookie_value(header: Option<&str>, name: &str) -> Option<String> {
    for part in header.unwrap_or("").split(';') {
        if let Some(index) = part.find('=')
            && index > 0
            && part[..index].trim() == name
        {
            let value = part[index + 1..].trim();
            return (!value.is_empty()).then(|| value.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ConnectDeviceKind::{Cookie, Native};
    use std::sync::atomic::{AtomicI64, Ordering};

    struct Setup {
        dir: tempfile::TempDir,
        clock: Arc<AtomicI64>,
    }

    impl Setup {
        fn new() -> Self {
            Setup {
                dir: tempfile::tempdir().unwrap(),
                clock: Arc::new(AtomicI64::new(1_000_000)),
            }
        }
        fn file(&self) -> PathBuf {
            self.dir.path().join("connect-sessions.json")
        }
        fn make(&self) -> Sessions {
            let clock = self.clock.clone();
            Sessions::with_clock(self.file(), Arc::new(move || clock.load(Ordering::SeqCst)))
        }
        fn advance(&self, ms: i64) {
            self.clock.fetch_add(ms, Ordering::SeqCst);
        }
    }

    #[test]
    fn swaps_a_code_for_a_session_once() {
        let setup = Setup::new();
        let sessions = setup.make();
        let code = sessions.issue_code("Laptop");
        let entered = sessions.enter(&code, Cookie).unwrap();
        assert_eq!(sessions.enter(&code, Cookie), None);
        assert!(sessions.check(Some(&entered.token), Cookie).is_some());
        let devices = sessions.list();
        assert_eq!(devices.len(), 1);
        assert_eq!(
            (devices[0].name.as_str(), devices[0].kind),
            ("Laptop", Cookie)
        );
    }

    #[test]
    fn takes_a_session_only_the_way_it_was_given_out() {
        let setup = Setup::new();
        let sessions = setup.make();
        let native = sessions
            .enter(&sessions.issue_code("iPad"), Native)
            .unwrap();
        assert!(sessions.check(Some(&native.token), Native).is_some());
        assert_eq!(sessions.check(Some(&native.token), Cookie), None);
        let cookie = sessions
            .enter(&sessions.issue_code("Laptop"), Cookie)
            .unwrap();
        assert_eq!(sessions.check(Some(&cookie.token), Native), None);
        let devices: Vec<_> = sessions
            .list()
            .into_iter()
            .map(|d| (d.name, d.kind))
            .collect();
        assert_eq!(
            devices,
            vec![("iPad".to_string(), Native), ("Laptop".to_string(), Cookie)]
        );
    }

    #[test]
    fn refuses_codes_after_a_minute_and_guesses() {
        let setup = Setup::new();
        let sessions = setup.make();
        let code = sessions.issue_code("Laptop");
        setup.advance(61_000);
        assert_eq!(sessions.enter(&code, Cookie), None);
        assert_eq!(sessions.enter("guess", Cookie), None);
        assert_eq!(sessions.check(Some("guess"), Cookie), None);
        assert_eq!(sessions.check(None, Cookie), None);
    }

    #[test]
    fn keeps_only_hashes_and_sessions_but_not_codes_across_restarts() {
        let setup = Setup::new();
        let before = setup.make().issue_code("Laptop");
        assert_eq!(setup.make().enter(&before, Cookie), None);
        let sessions = setup.make();
        let entered = sessions
            .enter(&sessions.issue_code("Laptop"), Cookie)
            .unwrap();
        assert!(
            !std::fs::read_to_string(setup.file())
                .unwrap()
                .contains(&entered.token)
        );
        assert!(setup.make().check(Some(&entered.token), Cookie).is_some());
    }

    #[test]
    fn reads_sessions_the_node_app_kept() {
        let setup = Setup::new();
        let token = "a-cookie-token";
        std::fs::write(
            setup.file(),
            format!(
                r#"[
  {{ "id": "old", "hash": "{}", "name": "Laptop", "createdAt": 1000000, "lastSeenAt": 1000000, "expiresAt": 9000000000000 }},
  {{ "id": "gone", "hash": "x", "name": "Phone", "createdAt": 1, "lastSeenAt": 1, "expiresAt": 2, "kind": "native" }}
]"#,
                hash(token)
            ),
        )
        .unwrap();
        let sessions = setup.make();
        assert_eq!(sessions.check(Some(token), Cookie).as_deref(), Some("old"));
        assert_eq!(
            sessions.list(),
            vec![ConnectDevice {
                id: "old".into(),
                name: "Laptop".into(),
                kind: Cookie,
                created_at: "1970-01-01T00:16:40.000Z".into(),
                last_seen_at: "1970-01-01T00:16:40.000Z".into(),
            }]
        );
    }

    #[test]
    fn ends_sessions_after_30_days() {
        let setup = Setup::new();
        let sessions = setup.make();
        let cookie = sessions
            .enter(&sessions.issue_code("Laptop"), Cookie)
            .unwrap();
        let native = sessions
            .enter(&sessions.issue_code("iPad"), Native)
            .unwrap();
        setup.advance(SESSION_LIFETIME + 1);
        assert_eq!(sessions.check(Some(&cookie.token), Cookie), None);
        assert_eq!(sessions.check(Some(&native.token), Native), None);
        assert_eq!(sessions.list(), vec![]);
    }

    #[test]
    fn signs_devices_out_and_says_which() {
        let setup = Setup::new();
        let sessions = setup.make();
        let ended: Arc<Mutex<Vec<Vec<String>>>> = Arc::default();
        let record = ended.clone();
        sessions.on_revoke(Arc::new(move |ids| {
            record.lock().unwrap().push(ids.to_vec())
        }));
        let laptop = sessions
            .enter(&sessions.issue_code("Laptop"), Cookie)
            .unwrap();
        let office = sessions
            .enter(&sessions.issue_code("Office"), Cookie)
            .unwrap();
        let laptop_id = sessions.check(Some(&laptop.token), Cookie).unwrap();
        sessions.revoke(&laptop_id);
        assert_eq!(sessions.check(Some(&laptop.token), Cookie), None);
        assert!(sessions.check(Some(&office.token), Cookie).is_some());
        let code = sessions.issue_code("Phone");
        sessions.revoke_all();
        assert_eq!(sessions.check(Some(&office.token), Cookie), None);
        assert_eq!(sessions.enter(&code, Cookie), None);
        let ended = ended.lock().unwrap();
        assert_eq!(ended.len(), 2);
        assert_eq!(ended[0], vec![laptop_id]);
        assert_eq!(ended[1].len(), 1);
    }

    #[test]
    fn finds_the_session_of_a_request_by_bearer_or_cookie() {
        let setup = Setup::new();
        let sessions = setup.make();
        let native = sessions
            .enter(&sessions.issue_code("iPad"), Native)
            .unwrap();
        let cookie = sessions
            .enter(&sessions.issue_code("Laptop"), Cookie)
            .unwrap();
        let bearer = format!("Bearer {}", native.token);
        let jar = format!("theme=dark; {SESSION_COOKIE}={}", cookie.token);
        assert!(sessions.session_of(Some(&bearer), None).is_some());
        assert!(sessions.session_of(None, Some(&jar)).is_some());
        // A Bearer header that isn't a session doesn't fall back to the cookie, nor is a token taken the other way.
        assert_eq!(sessions.session_of(Some("Bearer wrong"), Some(&jar)), None);
        assert_eq!(
            sessions.session_of(Some(&format!("Bearer {}", cookie.token)), None),
            None
        );
        assert_eq!(
            sessions.session_of(None, Some(&format!("{SESSION_COOKIE}={}", native.token))),
            None
        );
        assert_eq!(bearer_token(Some("Bearer a b")), None);
        assert_eq!(bearer_token(Some("Basic abc")), None);
        assert_eq!(cookie_value(Some("a=1; b= 2 "), "b").as_deref(), Some("2"));
        assert_eq!(cookie_value(Some("a=; =x"), "a"), None);
    }
}
