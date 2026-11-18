// The port of http.test.ts.

use std::sync::atomic::{AtomicBool, Ordering};

use futures::StreamExt;
use linker_edit_protocol::{LibraryFolder, LibraryKind, ResolveStatus};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest};

use super::*;
use crate::app::{AppOptions, SessionRequest, create_app, encode_uri_component};
use crate::library::folders::memory_library_store;
use crate::library::scan::{Probe as FileProbe, ProbeFile};
use crate::library::testing::link;
use crate::library::{Library, LibraryOptions};
use crate::projects::store::LoadedProject;
use crate::projects::testing::{memory_project_store, workspace_project};
use crate::providers::types::{ProviderSession, TurnSink};
use crate::remote::Hello;

// --- Helpers ---

/// A session that does nothing.
struct Quiet;

#[async_trait]
impl ProviderSession for Quiet {
    async fn send(
        &self,
        _: &str,
        _: Arc<dyn TurnSink>,
        _: CancellationToken,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    fn resume_id(&self) -> Option<String> {
        None
    }
}

/// An app with one project in dir, its agent doing nothing and Resolve not running.
fn app_options(dir: &Path) -> AppOptions {
    let project = SavedProjectIn(dir).project();
    AppOptions::new(
        "0.0.0",
        Arc::new(memory_project_store(vec![LoadedProject {
            project,
            threads: Vec::new(),
        }])),
        Arc::new(|_: SessionRequest| Arc::new(Quiet) as Arc<dyn ProviderSession>),
        Arc::new(|| Box::pin(async { ResolveStatus::NotRunning })),
    )
}

struct SavedProjectIn<'a>(&'a Path);

impl SavedProjectIn<'_> {
    fn project(&self) -> crate::projects::store::SavedProject {
        crate::projects::store::SavedProject {
            dir: self.0.to_path_buf(),
            ..workspace_project()
        }
    }
}

/// A probe that measures nothing.
struct NoProbe;

#[async_trait]
impl ProbeFile for NoProbe {
    async fn probe(&self, _: &Path, _: bool) -> anyhow::Result<FileProbe> {
        Ok(FileProbe {
            duration: None,
            channels: None,
            loudness: None,
        })
    }
}

fn library_with(path: &Path, cache: &Path) -> Arc<Library> {
    Arc::new(Library::new(LibraryOptions {
        store: Arc::new(memory_library_store(vec![LibraryFolder {
            id: "lib".into(),
            kind: LibraryKind::Music,
            label: "Music".into(),
            path: path.to_string_lossy().into_owned(),
            note: String::new(),
        }])),
        cache_dir: cache.to_path_buf(),
        probe: Arc::new(NoProbe),
        on_error: None,
    }))
}

/// The gate a test opens and closes, as remote access goes on and off.
#[derive(Default)]
struct Gate(Mutex<Option<RemoteGate>>);

#[async_trait]
impl RemoteRoutes for Gate {
    fn gate(&self) -> Option<RemoteGate> {
        self.0.lock().unwrap().clone()
    }
    async fn computers(&self) -> Vec<Computer> {
        Vec::new()
    }
}

impl Gate {
    fn set(&self, on: bool) {
        *self.0.lock().unwrap() = on.then(|| RemoteGate {
            host: "studio.tail1234.ts.net:8443".into(),
            owner: "jane@example.com".into(),
        });
    }
}

/// Serves the router on a port of its own.
async fn serve(routes: Router) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, routes).await });
    port
}

struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Answer {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// An HTTP/1.1 request as typed: the path reaches the server unnormalized, and any Host goes.
async fn request(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> Answer {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut text = format!("{method} {path} HTTP/1.1\r\n");
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("host"))
    {
        text.push_str(&format!("host: 127.0.0.1:{port}\r\n"));
    }
    for (name, value) in headers {
        text.push_str(&format!("{name}: {value}\r\n"));
    }
    text.push_str("connection: close\r\n");
    if let Some(body) = body {
        text.push_str(&format!("content-length: {}\r\n", body.len()));
    }
    text.push_str("\r\n");
    text.push_str(body.unwrap_or(""));
    stream.write_all(text.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let raw = String::from_utf8_lossy(&raw).into_owned();
    let (head, rest) = raw.split_once("\r\n\r\n").unwrap();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_lowercase(), value.trim().to_string()))
        .collect();
    let chunked = headers
        .iter()
        .any(|(name, value)| name == "transfer-encoding" && value.contains("chunked"));
    let body = if chunked {
        unchunk(rest)
    } else {
        rest.to_string()
    };
    Answer {
        status,
        headers,
        body,
    }
}

fn unchunk(mut rest: &str) -> String {
    let mut body = String::new();
    while let Some((size, after)) = rest.split_once("\r\n") {
        let size = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        body.push_str(&after[..size]);
        rest = &after[size + 2..];
    }
    body
}

async fn get(port: u16, path: &str, headers: &[(&str, &str)]) -> Answer {
    request(port, "GET", path, headers, None).await
}

async fn post(port: u16, path: &str, body: &Value, token: Option<&str>) -> Answer {
    let auth = token.map(|token| format!("Bearer {token}"));
    let headers: Vec<(&str, &str)> = auth
        .iter()
        .map(|auth| ("authorization", auth.as_str()))
        .collect();
    request(port, "POST", path, &headers, Some(&body.to_string())).await
}

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>;

/// Opens /ws; the socket, or the status it was turned away with.
async fn socket(port: u16, origin: Option<&str>, headers: &[(&str, &str)]) -> Result<Socket, u16> {
    let mut request = format!("ws://127.0.0.1:{port}/ws")
        .into_client_request()
        .unwrap();
    for (name, value) in headers {
        request.headers_mut().insert(
            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    if let Some(origin) = origin {
        request
            .headers_mut()
            .insert("origin", HeaderValue::from_str(origin).unwrap());
    }
    match tokio_tungstenite::connect_async(request).await {
        Ok((socket, _)) => Ok(socket),
        Err(tungstenite::Error::Http(response)) => Err(response.status().as_u16()),
        Err(error) => panic!("couldn't connect: {error}"),
    }
}

/// Waits until the server closes the socket.
async fn closed(socket: &mut Socket) {
    loop {
        match socket.next().await {
            Some(Ok(tungstenite::Message::Close(_))) | None | Some(Err(_)) => return,
            Some(Ok(_)) => {}
        }
    }
}

/// A built UI next to files it must never serve: root/build, root/build-secret, root/outside.txt.
fn ui() -> tempfile::TempDir {
    let root = tempfile::Builder::new()
        .prefix("linker-edit-http-")
        .tempdir()
        .unwrap();
    let dist = root.path().join("build");
    std::fs::create_dir_all(dist.join("_app")).unwrap();
    std::fs::write(dist.join("index.html"), "<html>app</html>").unwrap();
    std::fs::write(dist.join("_app").join("app.js"), "app()").unwrap();
    std::fs::create_dir_all(root.path().join("build-secret")).unwrap();
    std::fs::write(root.path().join("build-secret").join("token.txt"), "secret").unwrap();
    std::fs::write(root.path().join("outside.txt"), "outside").unwrap();
    root
}

const OWNER: [(&str, &str); 3] = [
    ("host", "studio.tail1234.ts.net:8443"),
    ("x-forwarded-for", "100.64.0.2"),
    ("tailscale-user-login", "jane@example.com"),
];

fn with<'a>(base: &[(&'a str, &'a str)], more: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut all: Vec<(&str, &str)> = base
        .iter()
        .filter(|(name, _)| !more.iter().any(|(other, _)| other == name))
        .copied()
        .collect();
    all.extend_from_slice(more);
    all
}

// --- isAllowedOrigin, byteRange, staticFile ---

#[test]
fn allows_only_this_apps_own_pages_on_the_socket() {
    let cases: [(Option<&str>, &str, bool); 12] = [
        (None, "127.0.0.1:4317", true),
        (Some("http://localhost:5317"), "localhost:5317", true),
        (Some("http://127.0.0.1:4317"), "127.0.0.1:4317", true),
        (Some("http://[::1]:4317"), "[::1]:4317", true),
        (Some("https://localhost"), "localhost", true),
        (Some("http://localhost:3000"), "127.0.0.1:4317", false),
        (Some("http://127.0.0.1:3000"), "127.0.0.1:4317", false),
        (Some("https://evil.example"), "127.0.0.1:4317", false),
        (
            Some("http://localhost.evil.example"),
            "localhost.evil.example",
            false,
        ),
        (
            Some("http://127.0.0.1.nip.io:4317"),
            "127.0.0.1.nip.io:4317",
            false,
        ),
        (Some("null"), "127.0.0.1:4317", false),
        (Some("file://"), "127.0.0.1:4317", false),
    ];
    for (origin, host, expected) in cases {
        assert_eq!(
            is_allowed_origin(origin, Some(host)),
            expected,
            "{origin:?} on {host}"
        );
    }
}

#[test]
fn reads_the_bytes_a_range_header_asks_for() {
    let part = |start, end| ByteRange::Part { start, end };
    let cases = [
        (None, ByteRange::Whole),
        (Some("bytes=0-"), part(0, 999)),
        (Some("bytes=100-199"), part(100, 199)),
        (Some("bytes=900-5000"), part(900, 999)),
        (Some("bytes=-100"), part(900, 999)),
        (Some("bytes=1000-"), ByteRange::Invalid),
        (Some("bytes=5-2"), ByteRange::Invalid),
        (Some("bytes=0-1,5-9"), ByteRange::Whole),
        (Some("items=0-1"), ByteRange::Whole),
    ];
    for (header, expected) in cases {
        assert_eq!(byte_range(header, 1000), expected, "{header:?}");
    }
}

#[test]
fn serves_built_files_and_index_html_for_everything_else() {
    let root = ui();
    let dist = root.path().join("build");
    let cases = [
        ("/", "index.html"),
        ("/_app/app.js", "_app/app.js"),
        ("/_app", "index.html"),
        ("/threads/42", "index.html"),
        ("/../outside.txt", "index.html"),
        ("/../build-secret/token.txt", "index.html"),
        ("\\..\\outside.txt", "index.html"),
    ];
    for (url_path, expected) in cases {
        assert_eq!(
            static_file(&dist, url_path),
            Some(js::join(&dist, expected)),
            "{url_path}"
        );
    }
}

#[test]
fn has_no_file_when_the_ui_isnt_built() {
    let root = ui();
    assert_eq!(static_file(&root.path().join("missing"), "/"), None);
}

#[tokio::test]
async fn serves_the_build_inside_the_binary_or_says_it_isnt_built() {
    let root = ui();
    let app = create_app(app_options(root.path()));
    let port = serve(router(HttpOptions::new(app, Web::Embedded, "secret"))).await;
    let answer = get(port, "/threads/42", &[]).await;
    match web_file("index.html") {
        Some(index) => {
            assert_eq!(answer.status, 200);
            assert_eq!(answer.body.as_bytes(), &*index);
            assert_eq!(answer.header("content-type"), Some("text/html"));
        }
        None => {
            assert_eq!(answer.status, 404);
            assert!(
                answer.body.contains("UI isn&#39;t built"),
                "{}",
                answer.body
            );
        }
    }
}

// --- server ---

mod server {
    use super::*;

    struct FakeLinker;

    #[async_trait]
    impl LinkerRoutes for FakeLinker {
        async fn start(&self) -> anyhow::Result<String> {
            Ok("https://clerk.example/oauth/authorize?state=abc".into())
        }
        async fn finish(&self, query: &HashMap<String, String>) -> anyhow::Result<String> {
            if query.get("state").map(String::as_str) != Some("abc") {
                anyhow::bail!("Not <this> one.");
            }
            Ok("Jane <Doe>".into())
        }
    }

    struct Server {
        port: u16,
        gate: Arc<Gate>,
        _root: tempfile::TempDir,
    }

    async fn start() -> Server {
        let root = ui();
        let app = create_app(app_options(root.path()));
        let gate = Arc::new(Gate::default());
        let mut options = HttpOptions::new(app, Web::Dir(root.path().join("build")), "secret");
        options.remote = gate.clone();
        options.linker = Some(Arc::new(FakeLinker));
        let port = serve(router(options)).await;
        Server {
            port,
            gate,
            _root: root,
        }
    }

    #[tokio::test]
    async fn takes_the_agents_calls_only_with_their_token() {
        let server = start().await;
        let port = server.port;
        let files = json!({ "files": ["C:\\Footage\\a.mov"] });
        assert_eq!(
            post(port, "/api/transcribe", &files, None).await.status,
            403
        );
        assert_eq!(
            post(port, "/api/transcribe", &files, Some("guess"))
                .await
                .status,
            403
        );
        let answer = post(port, "/api/transcribe", &files, Some("secret")).await;
        assert_eq!(
            (answer.status, answer.body.as_str()),
            (400, r#"{"error":"Transcription is off."}"#)
        );
        let call = json!({ "tool": "list_timelines", "input": {} });
        assert_eq!(
            post(port, "/api/resolve-tool", &call, None).await.status,
            403
        );
        let answer = post(port, "/api/resolve-tool", &call, Some("secret")).await;
        assert_eq!(
            (answer.status, answer.body.as_str()),
            (
                200,
                r#"{"text":"There is no Resolve tool called list_timelines.","isError":true}"#
            )
        );
    }

    #[tokio::test]
    async fn signs_in_with_linker_on_this_computer_only() {
        let server = start().await;
        let port = server.port;
        let start = get(port, "/api/linker/sign-in", &[]).await;
        assert_eq!(start.status, 302);
        assert_eq!(
            start.header("location"),
            Some("https://clerk.example/oauth/authorize?state=abc")
        );

        let done = get(port, "/api/linker/callback?code=x&state=abc", &[]).await;
        assert_eq!(done.status, 200);
        assert!(done.body.contains("Signed in as Jane &lt;Doe&gt;"));
        let refused = get(port, "/api/linker/callback?code=x&state=forged", &[]).await;
        assert_eq!(refused.status, 400);
        assert!(refused.body.contains("Not &lt;this&gt; one."));

        server.gate.set(true);
        assert_eq!(get(port, "/api/linker/sign-in", &OWNER).await.status, 403);
    }

    #[tokio::test]
    async fn answers_a_malformed_url_with_400_and_keeps_serving() {
        let server = start().await;
        assert_eq!(get(server.port, "/%", &[]).await.status, 400);
        let answer = get(server.port, "/", &[]).await;
        assert_eq!(
            (answer.status, answer.body.as_str()),
            (200, "<html>app</html>")
        );
    }

    #[tokio::test]
    async fn lets_only_the_owners_devices_in_through_tailscale_and_only_while_remote_access_is_on()
    {
        let server = start().await;
        let port = server.port;
        assert_eq!(
            get(port, "/", &[("host", "evil.example")]).await.status,
            403
        );
        assert_eq!(get(port, "/", &OWNER).await.status, 403);

        server.gate.set(true);
        let answer = get(port, "/", &OWNER).await;
        assert_eq!(
            (answer.status, answer.body.as_str()),
            (200, "<html>app</html>")
        );
        let bob = with(&OWNER, &[("tailscale-user-login", "bob@example.com")]);
        assert_eq!(get(port, "/", &bob).await.status, 403);
        assert!(
            socket(port, Some("https://studio.tail1234.ts.net:8443"), &OWNER)
                .await
                .is_ok()
        );
        assert!(
            socket(port, Some("https://evil.example"), &OWNER)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn refuses_websocket_connections_from_other_sites() {
        let server = start().await;
        for origin in ["https://evil.example", "http://localhost:3000"] {
            assert_eq!(
                socket(server.port, Some(origin), &[]).await.err(),
                Some(401),
                "{origin}"
            );
        }
    }

    #[tokio::test]
    async fn sends_the_apps_own_pages_a_snapshot() {
        let server = start().await;
        let origin = format!("http://127.0.0.1:{}", server.port);
        let mut socket = socket(server.port, Some(&origin), &[]).await.unwrap();
        let Some(Ok(tungstenite::Message::Text(text))) = socket.next().await else {
            panic!("no snapshot");
        };
        let message: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(message["type"], "snapshot");
        assert_eq!(message["state"]["busy"], false);
        assert_eq!(message["state"]["items"], json!([]));
    }
}

// --- Linker Connect ---

mod linker_connect {
    use super::*;

    const TUNNEL: &str = "edit-0123456789abcdef.linker.example";
    const VIA_TUNNEL: [(&str, &str); 3] = [
        ("host", TUNNEL),
        ("cf-connecting-ip", "203.0.113.9"),
        ("x-forwarded-for", "203.0.113.9"),
    ];

    struct FakeConnect {
        on: AtomicBool,
        sessions: Arc<Sessions>,
        opened: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ConnectRoutes for FakeConnect {
        fn host(&self) -> Option<String> {
            self.on.load(Ordering::SeqCst).then(|| TUNNEL.to_string())
        }
        async fn admit(&self, token: &str, device: &str) -> Result<String, Refusal> {
            if token == "jane" {
                Ok(self.sessions.issue_code(device))
            } else {
                Err(Refusal {
                    status: 403,
                    error: "Not yours.".into(),
                })
            }
        }
        fn redeem(&self, code: &str) -> Result<Redeemed, Refusal> {
            match self.sessions.enter(code, ConnectDeviceKind::Native) {
                Some(entered) => Ok(Redeemed {
                    session: entered.token,
                    expires_at: js::iso_from_ms(entered.expires_at),
                }),
                None => Err(Refusal {
                    status: 401,
                    error: "Ran out.".into(),
                }),
            }
        }
        async fn open(&self, url: &str) -> anyhow::Result<String> {
            self.opened.lock().unwrap().push(url.to_string());
            Ok(format!("{url}/api/connect/enter?code=x"))
        }
        async fn others(&self) -> Vec<LinkedComputer> {
            let computer = |id: &str, name: &str, url: &str, online| LinkedComputer {
                id: id.into(),
                name: name.into(),
                url: url.into(),
                online: Some(online),
                linked_at: String::new(),
                last_seen_at: None,
            };
            vec![
                computer(
                    "office",
                    "Office",
                    "https://edit-1111111111111111.linker.example",
                    true,
                ),
                computer(
                    "away",
                    "Away",
                    "https://edit-2222222222222222.linker.example",
                    false,
                ),
            ]
        }
    }

    struct FakeProbe;

    #[async_trait]
    impl Probe for FakeProbe {
        async fn probe(&self, url: &str) -> Option<Hello> {
            url.contains("1111").then(|| Hello {
                name: "Office PC".into(),
                version: "2026.1114.1".into(),
            })
        }
    }

    struct Server {
        port: u16,
        connect: Arc<FakeConnect>,
        sessions: Arc<Sessions>,
        _dirs: (tempfile::TempDir, tempfile::TempDir),
    }

    async fn start() -> Server {
        let root = ui();
        let dir = tempfile::Builder::new()
            .prefix("linker-edit-http-connect-")
            .tempdir()
            .unwrap();
        std::fs::write(dir.path().join("Song.mp3"), "0123456789").unwrap();
        let sessions = Arc::new(Sessions::new(dir.path().join("sessions.json")));
        let mut options = app_options(root.path());
        options.library = Some(library_with(dir.path(), &dir.path().join(".cache")));
        let app = create_app(options);
        let connect = Arc::new(FakeConnect {
            on: AtomicBool::new(true),
            sessions: sessions.clone(),
            opened: Mutex::default(),
        });
        let mut options = HttpOptions::new(app, Web::Dir(root.path().join("build")), "secret");
        options.connect = Some(ConnectSide {
            routes: connect.clone(),
            sessions: sessions.clone(),
            probe: Arc::new(FakeProbe),
        });
        let port = serve(router(options)).await;
        Server {
            port,
            connect,
            sessions,
            _dirs: (root, dir),
        }
    }

    fn tunnel(more: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
        with(&VIA_TUNNEL, &[])
            .into_iter()
            .map(|(name, value)| (name, value.to_string()))
            .filter(|(name, _)| !more.iter().any(|(other, _)| other == name))
            .chain(more.iter().map(|(name, value)| (*name, value.to_string())))
            .collect()
    }

    fn refs<'a>(headers: &'a [(&'static str, String)]) -> Vec<(&'static str, &'a str)> {
        headers
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .collect()
    }

    async fn call(
        port: u16,
        method: &str,
        path: &str,
        more: &[(&'static str, &str)],
        body: Option<&Value>,
    ) -> Answer {
        let headers = tunnel(more);
        let body = body.map(Value::to_string);
        request(port, method, path, &refs(&headers), body.as_deref()).await
    }

    /// Signs in as Linker-Edit's window does; answers its session cookie, `name=value`.
    async fn sign_in(port: u16) -> String {
        let admitted = call(
            port,
            "POST",
            "/api/connect/session",
            &[("authorization", "Bearer jane")],
            Some(&json!({ "name": "Laptop" })),
        )
        .await;
        assert_eq!(admitted.status, 200);
        let code: Value = serde_json::from_str(&admitted.body).unwrap();
        let entered = call(
            port,
            "GET",
            &format!("/api/connect/enter?code={}", code["code"].as_str().unwrap()),
            &[],
            None,
        )
        .await;
        assert_eq!(entered.status, 302);
        let cookie = entered.header("set-cookie").unwrap().to_string();
        assert!(
            cookie.contains("HttpOnly; Secure; SameSite=Lax"),
            "{cookie}"
        );
        cookie.split(';').next().unwrap().to_string()
    }

    /// Signs in as the iPad app does; answers its Bearer session.
    async fn sign_in_native(port: u16) -> String {
        let admitted = call(
            port,
            "POST",
            "/api/connect/session",
            &[("authorization", "Bearer jane")],
            Some(&json!({ "name": "iPad" })),
        )
        .await;
        let code: Value = serde_json::from_str(&admitted.body).unwrap();
        let body = json!({ "code": code["code"] });
        let redeemed = call(port, "POST", "/api/connect/native", &[], Some(&body)).await;
        assert_eq!(redeemed.status, 200);
        assert_eq!(redeemed.header("cache-control"), Some("no-store"));
        assert_eq!(redeemed.header("set-cookie"), None);
        let answer: Map<String, Value> = serde_json::from_str(&redeemed.body).unwrap();
        let mut keys: Vec<&String> = answer.keys().collect();
        keys.sort();
        assert_eq!(keys, ["expiresAt", "session"]);
        let expires = crate::thread::parse_ms(answer["expiresAt"].as_str().unwrap()).unwrap();
        assert!(expires > js::now_millis());
        // The code is spent.
        assert_eq!(
            call(port, "POST", "/api/connect/native", &[], Some(&body))
                .await
                .status,
            401
        );
        answer["session"].as_str().unwrap().to_string()
    }

    /// Opens /ws through the tunnel; the socket when it was let in.
    async fn open_socket(
        port: u16,
        more: &[(&'static str, &str)],
        origin: Option<&str>,
    ) -> Option<Socket> {
        let headers = tunnel(more);
        socket(port, origin, &refs(&headers)).await.ok()
    }

    #[tokio::test]
    async fn lets_nothing_through_the_tunnel_without_a_session_but_the_way_in() {
        let server = start().await;
        let port = server.port;
        assert_eq!(call(port, "GET", "/", &[], None).await.status, 401);
        assert_eq!(call(port, "GET", "/media/x", &[], None).await.status, 401);
        assert_eq!(call(port, "GET", "/api/hello", &[], None).await.status, 200);
        assert_eq!(
            call(port, "POST", "/api/connect/session", &[], Some(&json!({})))
                .await
                .status,
            401
        );
        let bob = call(
            port,
            "POST",
            "/api/connect/session",
            &[("authorization", "Bearer bob")],
            Some(&json!({})),
        )
        .await;
        assert_eq!(bob.status, 403);
        assert_eq!(
            call(port, "GET", "/api/connect/enter?code=guess", &[], None)
                .await
                .status,
            401
        );
        // Opening other computers and signing in with Linker are for this computer itself.
        assert_eq!(
            call(
                port,
                "POST",
                "/api/connect/open",
                &[],
                Some(&json!({ "url": "https://x" }))
            )
            .await
            .status,
            401
        );
        assert_eq!(
            call(port, "GET", "/api/linker/sign-in", &[], None)
                .await
                .status,
            401
        );
    }

    #[tokio::test]
    async fn serves_the_app_to_a_session_and_cuts_its_connections_off_when_its_signed_out() {
        let server = start().await;
        let port = server.port;
        let cookie = sign_in(port).await;
        let answer = call(port, "GET", "/", &[("cookie", &cookie)], None).await;
        assert_eq!(
            (answer.status, answer.body.as_str()),
            (200, "<html>app</html>")
        );
        let open = call(
            port,
            "POST",
            "/api/connect/open",
            &[("cookie", &cookie)],
            Some(&json!({ "url": "https://x" })),
        )
        .await;
        assert_eq!(open.status, 403);

        let headers = tunnel(&[("cookie", &cookie)]);
        assert_eq!(
            socket(port, Some("https://evil.example"), &refs(&headers))
                .await
                .err(),
            Some(401)
        );

        let mut own = socket(port, Some(&format!("https://{TUNNEL}")), &refs(&headers))
            .await
            .unwrap();
        server.sessions.revoke_all();
        closed(&mut own).await;
        assert_eq!(
            call(port, "GET", "/", &[("cookie", &cookie)], None)
                .await
                .status,
            401
        );
    }

    #[tokio::test]
    async fn gives_the_ipad_app_a_bearer_session_that_opens_everything_a_cookie_does() {
        let server = start().await;
        let port = server.port;
        let session = sign_in_native(port).await;
        let bearer = format!("Bearer {session}");
        let auth: [(&'static str, &str); 1] = [("authorization", &bearer)];
        let answer = call(port, "GET", "/", &auth, None).await;
        assert_eq!(
            (answer.status, answer.body.as_str()),
            (200, "<html>app</html>")
        );
        // AVPlayer sends the header with its Range requests.
        let part = call(
            port,
            "GET",
            "/library/lib/Song.mp3",
            &[("authorization", &bearer), ("range", "bytes=2-4")],
            None,
        )
        .await;
        assert_eq!((part.status, part.body.as_str()), (206, "234"));
        assert!(
            server
                .sessions
                .list()
                .iter()
                .any(|device| device.name == "iPad" && device.kind == ConnectDeviceKind::Native)
        );

        // A native app sends no Origin; a page can't add the header to a WebSocket, but if one names another site, no.
        let mut native = open_socket(port, &auth, None)
            .await
            .expect("let in without an Origin");
        assert!(
            open_socket(port, &auth, Some(&format!("https://{TUNNEL}")))
                .await
                .is_some()
        );
        assert!(
            open_socket(port, &auth, Some("https://evil.example"))
                .await
                .is_none()
        );
        assert!(
            open_socket(port, &[("authorization", "Bearer guess")], None)
                .await
                .is_none()
        );

        // Signing the iPad out in Settings cuts it off at once.
        let id = server
            .sessions
            .list()
            .into_iter()
            .find(|device| device.kind == ConnectDeviceKind::Native)
            .unwrap()
            .id;
        server.sessions.revoke(&id);
        closed(&mut native).await;
        let after = call(port, "GET", "/", &auth, None).await;
        assert_eq!(after.status, 401);
        assert!(after.header("www-authenticate").unwrap().contains("Bearer"));
        let body: Value = serde_json::from_str(&after.body).unwrap();
        assert!(body["error"].is_string() && body.as_object().unwrap().len() == 1);
        assert!(open_socket(port, &auth, None).await.is_none());
    }

    #[tokio::test]
    async fn takes_a_session_only_the_way_it_was_given_out() {
        let server = start().await;
        let port = server.port;
        let session = sign_in_native(port).await;
        let cookie = sign_in(port).await;
        let as_cookie = format!("linker_edit_session={session}");
        assert_eq!(
            call(port, "GET", "/", &[("cookie", &as_cookie)], None)
                .await
                .status,
            401
        );
        let as_bearer = format!("Bearer {}", cookie.split_once('=').unwrap().1);
        assert_eq!(
            call(port, "GET", "/", &[("authorization", &as_bearer)], None)
                .await
                .status,
            401
        );
        // A Bearer header that isn't a session doesn't fall back to the cookie next to it.
        let both = call(
            port,
            "GET",
            "/",
            &[("cookie", &cookie), ("authorization", "Bearer guess")],
            None,
        )
        .await;
        assert_eq!(both.status, 401);
        // Without a session the native route answers nothing else.
        assert_eq!(
            call(port, "GET", "/api/connect/native", &[], None)
                .await
                .status,
            405
        );
        assert_eq!(
            call(
                port,
                "POST",
                "/api/connect/native",
                &[],
                Some(&json!({ "code": "guess" }))
            )
            .await
            .status,
            401
        );
        assert_eq!(
            call(port, "POST", "/api/connect/native", &[], Some(&json!({})))
                .await
                .status,
            401
        );
    }

    #[tokio::test]
    async fn takes_a_bearer_session_only_through_the_tunnel() {
        let server = start().await;
        let port = server.port;
        let bearer = format!("Bearer {}", sign_in_native(port).await);
        // Through Tailscale (off here) or with the tunnel's name but not through Cloudflare, it counts for nothing.
        let tailscale = with(&OWNER, &[("authorization", &bearer)]);
        assert_eq!(get(port, "/", &tailscale).await.status, 403);
        let named = [
            ("host", TUNNEL),
            ("x-forwarded-for", "203.0.113.9"),
            ("authorization", bearer.as_str()),
        ];
        assert_eq!(get(port, "/", &named).await.status, 403);
        // This computer itself is let in as before, whatever the header says.
        assert_eq!(
            get(port, "/", &[("authorization", "Bearer guess")])
                .await
                .status,
            200
        );
        // And the native route is only for the tunnel.
        let native = request(
            port,
            "POST",
            "/api/connect/native",
            &[("host", TUNNEL), ("x-forwarded-for", "203.0.113.9")],
            Some(r#"{"code":"x"}"#),
        )
        .await;
        assert_eq!(native.status, 403);
    }

    #[tokio::test]
    async fn turns_the_tunnel_away_while_linker_connect_is_off() {
        let server = start().await;
        let port = server.port;
        let cookie = sign_in(port).await;
        let bearer = format!("Bearer {}", sign_in_native(port).await);
        server.connect.on.store(false, Ordering::SeqCst);
        assert_eq!(
            call(port, "GET", "/", &[("cookie", &cookie)], None)
                .await
                .status,
            403
        );
        assert_eq!(
            call(port, "GET", "/", &[("authorization", &bearer)], None)
                .await
                .status,
            403
        );
    }

    #[tokio::test]
    async fn lists_linked_computers_and_opens_them_only_for_this_computer() {
        let server = start().await;
        let port = server.port;
        let list = get(port, "/api/computers", &[]).await;
        let list: Value = serde_json::from_str(&list.body).unwrap();
        assert_eq!(
            list["others"],
            json!([
                { "name": "Office PC", "url": "https://edit-1111111111111111.linker.example", "version": "2026.1114.1", "via": "linker", "online": true },
                { "name": "Away", "url": "https://edit-2222222222222222.linker.example", "version": "", "via": "linker", "online": false },
            ])
        );
        let open = request(
            port,
            "POST",
            "/api/connect/open",
            &[("content-type", "application/json")],
            Some(r#"{"url":"https://edit-1111111111111111.linker.example"}"#),
        )
        .await;
        assert_eq!(
            serde_json::from_str::<Value>(&open.body).unwrap(),
            json!({ "url": "https://edit-1111111111111111.linker.example/api/connect/enter?code=x" })
        );
        let from_website = request(
            port,
            "POST",
            "/api/connect/open",
            &[("origin", "https://evil.example")],
            Some(r#"{"url":"https://x"}"#),
        )
        .await;
        assert_eq!(from_website.status, 403);
        assert_eq!(
            *server.connect.opened.lock().unwrap(),
            ["https://edit-1111111111111111.linker.example"]
        );
    }
}

// --- library files ---

mod library_files {
    use super::*;

    struct Server {
        port: u16,
        gate: Arc<Gate>,
        outside: tempfile::TempDir,
        _library: tempfile::TempDir,
        _root: tempfile::TempDir,
    }

    async fn start() -> Server {
        let make = |prefix: &str| tempfile::Builder::new().prefix(prefix).tempdir().unwrap();
        let library = make("linker-edit-http-library-");
        let outside = make("linker-edit-http-outside-");
        std::fs::write(library.path().join("Song.mp3"), "0123456789").unwrap();
        std::fs::write(
            library.path().join("page.html"),
            "<script>alert(1)</script>",
        )
        .unwrap();
        std::fs::write(outside.path().join("Secret.mp3"), "secret").unwrap();
        assert!(link(outside.path(), &library.path().join("Out"), true));
        let root = ui();
        let mut options = app_options(Path::new("/work"));
        options.library = Some(library_with(library.path(), &library.path().join(".cache")));
        let app = create_app(options);
        let gate = Arc::new(Gate::default());
        let mut options = HttpOptions::new(app, Web::Dir(root.path().join("build")), "secret");
        options.remote = gate.clone();
        let port = serve(router(options)).await;
        Server {
            port,
            gate,
            outside,
            _library: library,
            _root: root,
        }
    }

    #[tokio::test]
    async fn plays_a_file_inside_a_library_folder_in_parts_when_asked_never_as_a_page() {
        let server = start().await;
        let whole = get(server.port, "/library/lib/Song.mp3", &[]).await;
        assert_eq!((whole.status, whole.body.as_str()), (200, "0123456789"));
        assert_eq!(whole.header("content-type"), Some("audio/mpeg"));
        assert_eq!(whole.header("x-content-type-options"), Some("nosniff"));
        assert!(
            whole
                .header("content-security-policy")
                .unwrap()
                .contains("sandbox")
        );
        let part = get(
            server.port,
            "/library/lib/Song.mp3",
            &[("range", "bytes=2-4")],
        )
        .await;
        assert_eq!((part.status, part.body.as_str()), (206, "234"));
    }

    #[tokio::test]
    async fn refuses_paths_out_of_the_folder_and_files_it_doesnt_play() {
        let server = start().await;
        let outside = server.outside.path();
        let base = outside.file_name().unwrap().to_string_lossy().into_owned();
        let secret = outside.join("Secret.mp3").to_string_lossy().into_owned();
        let paths = [
            format!("/library/lib/../{base}/Secret.mp3"),
            format!("/library/lib/%2e%2e/{base}/Secret.mp3"),
            format!("/library/lib/..%2F{base}%2FSecret.mp3"),
            format!("/library/lib/..%5C{base}%5CSecret.mp3"),
            "/library/lib/Out/Secret.mp3".into(),
            format!("/library/lib/{}", encode_uri_component(&secret)),
            "/library/lib/C:/Windows/Media/tada.wav".into(),
            "/library/lib/C%3A%5CWindows%5CMedia%5Ctada.wav".into(),
            "/library/lib/%5C%5Cserver%5Cshare%5CSong.mp3".into(),
            "/library/lib/page.html".into(),
            "/library/other/Song.mp3".into(),
            "/library/lib/".into(),
        ];
        for path in paths {
            let answer = get(server.port, &path, &[]).await;
            assert_eq!((answer.status, answer.body.as_str()), (404, ""), "{path}");
        }
    }

    #[tokio::test]
    async fn plays_only_to_this_computer_and_the_owners_own_devices() {
        let server = start().await;
        let port = server.port;
        assert_eq!(get(port, "/library/lib/Song.mp3", &OWNER).await.status, 403);
        assert_eq!(
            get(port, "/library/lib/Song.mp3", &[("host", "evil.example")])
                .await
                .status,
            403
        );
        server.gate.set(true);
        let answer = get(port, "/library/lib/Song.mp3", &OWNER).await;
        assert_eq!((answer.status, answer.body.as_str()), (200, "0123456789"));
        let bob = with(&OWNER, &[("tailscale-user-login", "bob@example.com")]);
        assert_eq!(get(port, "/library/lib/Song.mp3", &bob).await.status, 403);
    }

    #[tokio::test]
    async fn takes_the_agents_library_calls_only_with_their_token() {
        let server = start().await;
        let call = json!({ "tool": "library_search", "input": {} });
        assert_eq!(
            post(server.port, "/api/library-tool", &call, None)
                .await
                .status,
            403
        );
        assert_eq!(
            post(server.port, "/api/library-tool", &call, Some("guess"))
                .await
                .status,
            403
        );
        assert_eq!(
            post(server.port, "/api/library-tool", &call, Some("secret"))
                .await
                .status,
            200
        );
    }
}
