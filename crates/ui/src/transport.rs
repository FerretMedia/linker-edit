//! The `/ws` client: the window's [`Link`] to a server over a WebSocket, for a server that isn't
//! in this process (another computer's, or the TypeScript one during the port).
//!
//! It runs on a thread of its own with a small tokio runtime, since tokio-tungstenite needs tokio
//! and GPUI's executor isn't one. It feeds the same channels as [`Link::pair`], so the window
//! can't tell the two apart. Like connection.svelte.ts it reconnects a second after the socket
//! closes, says so with [`LinkEvent::Disconnected`], and drops commands sent while it is down
//! (a closed socket does too); the server sends a fresh snapshot on every connect.
//!
//! How it gets in, as server/http.ts lets clients in:
//! - [`Auth::Local`], this computer's own server on 127.0.0.1: nothing. The browser sends no
//!   token either; the server lets in a socket whose Host is a loopback address and whose Origin
//!   is that same address or missing, and a native client sends none.
//! - [`Auth::Bearer`], a Linker Connect session through the tunnel (docs/sync-plan.md, "Native
//!   clients"): `Authorization: Bearer <session>` on the upgrade, no Origin. A 401 means the
//!   session ended: [`LinkEvent::SignedOut`], and no more retries.

use std::time::Duration;

use futures::{SinkExt, StreamExt, channel::mpsc::UnboundedSender};
use tokio_tungstenite::tungstenite::{
    self, Message, client::IntoClientRequest, http::HeaderValue, http::StatusCode,
};

use crate::state::{ClientMessage, Link, LinkEvent, ServerAddress, ServerEnd, ServerMessage};

/// How long to wait before connecting again, as connection.svelte.ts's `setTimeout(…, 1000)`.
const RECONNECT: Duration = Duration::from_secs(1);

/// How the link proves it may come in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Auth {
    /// This computer's server, on a loopback address.
    Local,
    /// A Linker Connect session from `POST /api/connect/native`.
    Bearer(String),
}

/// `http://127.0.0.1:4317` → `ws://127.0.0.1:4317/ws`; `https://…` → `wss://…/ws`.
pub fn ws_url(server: &str) -> anyhow::Result<String> {
    let server = server.trim().trim_end_matches('/');
    let rest = if let Some(rest) = server.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = server.strip_prefix("http://") {
        format!("ws://{rest}")
    } else if server.starts_with("ws://") || server.starts_with("wss://") {
        server.to_string()
    } else {
        anyhow::bail!("{server} isn't an http(s) address");
    };
    Ok(if rest.ends_with("/ws") {
        rest
    } else {
        format!("{rest}/ws")
    })
}

/// A link to the server at `server` (its http(s) address), connecting in the background.
pub fn connect(server: &str, auth: Auth) -> anyhow::Result<Link> {
    let url = ws_url(server)?;
    let address = ServerAddress::new(server, auth.clone())?;
    let (link, end) = Link::pair();
    let link = link.with_server(address);
    std::thread::Builder::new()
        .name("linker-edit-ws".into())
        .spawn(move || {
            // rustls has no provider of its own when more than one is linked; pick one.
            let _ = rustls::crypto::ring::default_provider().install_default();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a tokio runtime for the socket");
            runtime.block_on(run(url, auth, end));
        })?;
    Ok(link)
}

async fn run(url: String, auth: Auth, end: ServerEnd) {
    let ServerEnd {
        events,
        mut commands,
    } = end;
    loop {
        // Sent while the socket was down: gone, as from a closed WebSocket.
        while commands.try_recv().is_ok() {}
        match connect_once(&url, &auth).await {
            Ok(socket) => {
                if events.unbounded_send(LinkEvent::Connected).is_err() {
                    return;
                }
                let window_open = pump(socket, &events, &mut commands).await;
                if !window_open || events.unbounded_send(LinkEvent::Disconnected).is_err() {
                    return;
                }
            }
            Err(tungstenite::Error::Http(response))
                if response.status() == StatusCode::UNAUTHORIZED =>
            {
                let _ = events.unbounded_send(LinkEvent::SignedOut);
                return;
            }
            Err(error) => eprintln!("Couldn't connect to {url}: {error}"),
        }
        tokio::time::sleep(RECONNECT).await;
        if events.is_closed() {
            return;
        }
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect_once(url: &str, auth: &Auth) -> Result<Socket, tungstenite::Error> {
    let mut request = url.into_client_request()?;
    if let Auth::Bearer(session) = auth {
        let value = HeaderValue::from_str(&format!("Bearer {session}"))
            .map_err(|error| tungstenite::Error::HttpFormat(error.into()))?;
        request.headers_mut().insert("authorization", value);
    }
    let (socket, _) = tokio_tungstenite::connect_async(request).await?;
    Ok(socket)
}

/// Passes messages both ways until the socket closes. False when the window is gone.
async fn pump(
    socket: Socket,
    events: &UnboundedSender<LinkEvent>,
    commands: &mut futures::channel::mpsc::UnboundedReceiver<ClientMessage>,
) -> bool {
    let (mut sink, mut stream) = socket.split();
    loop {
        tokio::select! {
            message = stream.next() => match message {
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<ServerMessage>(&text) {
                    Ok(message) => {
                        if events.unbounded_send(LinkEvent::Message(message)).is_err() {
                            return false;
                        }
                    }
                    // A newer server's message this window doesn't know yet.
                    Err(error) => eprintln!("Unknown server message ({error}): {:.200}", text.as_str()),
                },
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return true,
                Some(Ok(_)) => {}
            },
            command = commands.next() => match command {
                Some(command) => {
                    let text = serde_json::to_string(&command).expect("a ClientMessage serializes");
                    if sink.send(Message::text(text)).await.is_err() {
                        return true;
                    }
                }
                None => {
                    let _ = sink.close().await;
                    return false;
                }
            },
        }
    }
}

#[cfg(test)]
// tungstenite's handshake callbacks return its own large error type.
#[allow(clippy::result_large_err)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};

    #[test]
    fn makes_the_socket_address_from_the_server_address() {
        assert_eq!(
            ws_url("http://127.0.0.1:4521").unwrap(),
            "ws://127.0.0.1:4521/ws"
        );
        assert_eq!(
            ws_url("https://edit-0123456789abcdef.linker.cool/").unwrap(),
            "wss://edit-0123456789abcdef.linker.cool/ws"
        );
        assert_eq!(
            ws_url("ws://localhost:1/ws").unwrap(),
            "ws://localhost:1/ws"
        );
        assert!(ws_url("127.0.0.1:4521").is_err());
        // And where its files are, with the session only for its own addresses.
        let local = ServerAddress::new("http://127.0.0.1:4521/", Auth::Local).unwrap();
        assert_eq!(local.base, "http://127.0.0.1:4521");
        assert!(local.is_local());
        assert_eq!(local.url("/media/a"), "http://127.0.0.1:4521/media/a");
        let session = Auth::Bearer("s3ss10n".into());
        let connect =
            ServerAddress::new("wss://Edit-0123456789abcdef.linker.cool/ws", session).unwrap();
        assert_eq!(connect.base, "https://edit-0123456789abcdef.linker.cool");
        assert_eq!(connect.bearer(), Some("s3ss10n"));
        assert!(!connect.is_local());
        assert!(connect.serves("https://edit-0123456789abcdef.linker.cool/preview/a.mp4"));
        assert!(!connect.serves("https://linker.cool/avatar.png"));
        assert!(ServerAddress::new("127.0.0.1:4521", Auth::Local).is_err());
    }

    /// A server that checks the upgrade as server/http.ts does, then echoes a snapshot back.
    async fn server(
        expect_bearer: Option<&'static str>,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let mut received = Vec::new();
            // The first connection is dropped at once, to see the client come back.
            let (stream, _) = listener.accept().await.unwrap();
            drop(tokio_tungstenite::accept_async(stream).await.unwrap());
            let (stream, _) = listener.accept().await.unwrap();
            let check =
                |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
                    assert!(
                        request.headers().get("origin").is_none(),
                        "a native client sends no Origin"
                    );
                    let auth = request
                        .headers()
                        .get("authorization")
                        .map(|v| v.to_str().unwrap().to_string());
                    assert_eq!(auth, expect_bearer.map(|token| format!("Bearer {token}")));
                    Ok(response)
                };
            let mut socket = tokio_tungstenite::accept_hdr_async(stream, check)
                .await
                .unwrap();
            let snapshot = include_str!(
                "../../../apps/ipad/Packages/LinkerEditKit/Tests/Fixtures/server/001-snapshot.json"
            );
            socket.send(Message::text(snapshot)).await.unwrap();
            if let Some(Ok(Message::Text(text))) = socket.next().await {
                received.push(text.to_string());
            }
            received
        });
        (address, task)
    }

    #[tokio::test]
    async fn reconnects_and_carries_messages_both_ways() {
        let (address, server) = server(Some("s3ss10n")).await;
        let (link, end) = Link::pair();
        let url = ws_url(&address).unwrap();
        let client = tokio::spawn(run(url, Auth::Bearer("s3ss10n".into()), end));
        let Link {
            mut events,
            commands,
            ..
        } = link;
        let mut seen = Vec::new();
        while let Some(event) = events.next().await {
            let snapshot = matches!(event, LinkEvent::Message(ServerMessage::Snapshot { .. }));
            seen.push(match event {
                LinkEvent::Connected => "connected",
                LinkEvent::Disconnected => "disconnected",
                LinkEvent::SignedOut => "signed out",
                LinkEvent::Message(_) => "snapshot",
            });
            if snapshot {
                break;
            }
        }
        assert_eq!(seen, ["connected", "disconnected", "connected", "snapshot"]);
        commands.unbounded_send(ClientMessage::Stop).unwrap();
        assert_eq!(server.await.unwrap(), [r#"{"type":"stop"}"#]);
        drop(commands);
        drop(events);
        client.abort();
    }

    #[tokio::test]
    async fn stops_when_the_session_is_turned_away() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = ws_url(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let refuse = |_: &Request, _: Response| -> Result<Response, ErrorResponse> {
                let mut response = ErrorResponse::new(Some("Signed out".into()));
                *response.status_mut() = StatusCode::UNAUTHORIZED;
                Err(response)
            };
            let _ = tokio_tungstenite::accept_hdr_async(stream, refuse).await;
        });
        let (link, end) = Link::pair();
        run(url, Auth::Bearer("old".into()), end).await;
        let events: Vec<LinkEvent> = link.events.collect().await;
        assert!(matches!(events[..], [LinkEvent::SignedOut]));
    }
}
