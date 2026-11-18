//! The window's one HTTP client, reqwest, for what the browser fetches from its page's origin:
//!
//! - Pictures: GPUI's `img("http://…")` fetches through the app's HTTP client, which is a no-op
//!   until [`install`] sets this one (reqwest on a small tokio runtime of its own; GPUI's executor
//!   isn't tokio). The library page's thumbnails (/library/…), the history's clip tiles
//!   (/media/…), the Linker account's picture.
//! - A few blocking requests, made off the UI thread: the computer menu's list and sign-in, the
//!   preview's timeline ([`get`], [`client`]).
//!
//! A request to the server the window is on carries its Linker Connect session as Bearer, as the
//! browser sends its cookie; requests anywhere else (a picture on linker.cool) don't. Which server
//! that is follows [`crate::state::AppState`] ([`set_server`]), so it changes with the computer
//! menu.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use gpui::Global;
use gpui::http_client::{AsyncBody, HttpClient, Request, Response, Url, http::HeaderValue};

use crate::state::ServerAddress;

/// The server the window is on, shared with the picture client.
#[derive(Clone, Default)]
struct Current(Arc<Mutex<Option<ServerAddress>>>);

impl Global for Current {}

impl Current {
    fn bearer_for(&self, url: &str) -> Option<String> {
        let server = self.0.lock().ok()?;
        let server = server.as_ref().filter(|server| server.serves(url))?;
        server.bearer().map(str::to_string)
    }
}

struct Client {
    runtime: tokio::runtime::Runtime,
    client: reqwest::Client,
    current: Current,
}

/// Makes GPUI's `img` fetch over HTTP. Without it (tests) pictures stay blank.
pub fn install(cx: &mut gpui::App) -> anyhow::Result<()> {
    // rustls has no provider of its own when more than one is linked; pick one.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name("linker-edit-http")
        .enable_all()
        .build()?;
    let current = Current::default();
    cx.set_global(current.clone());
    cx.set_http_client(Arc::new(Client {
        runtime,
        client: reqwest::Client::new(),
        current,
    }));
    Ok(())
}

/// The window is now on `server`: requests to it carry its session.
pub fn set_server(server: Option<&ServerAddress>, cx: &mut gpui::App) {
    if let Some(current) = cx.try_global::<Current>()
        && let Ok(mut slot) = current.0.lock()
    {
        *slot = server.cloned();
    }
}

impl HttpClient for Client {
    fn user_agent(&self) -> Option<&HeaderValue> {
        None
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }

    fn send(
        &self,
        request: Request<AsyncBody>,
    ) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        let url = request.uri().to_string();
        let mut builder = self.client.request(request.method().clone(), &url);
        if let Some(session) = self.current.bearer_for(&url) {
            builder = builder.bearer_auth(session);
        }
        // Only GETs go through here (pictures), so the request's body is never sent.
        let task = self.runtime.spawn(async move {
            let response = builder.send().await?;
            let status = response.status();
            let headers = response.headers().clone();
            let body = response.bytes().await?;
            anyhow::Ok((status, headers, body))
        });
        async move {
            let (status, headers, body) = task.await??;
            let mut response = Response::builder().status(status);
            if let Some(map) = response.headers_mut() {
                *map = headers;
            }
            Ok(response.body(AsyncBody::from(body.to_vec()))?)
        }
        .boxed()
    }
}

/// A blocking client that gives up after `timeout`. Never on the UI thread.
pub fn client(timeout: Duration) -> Result<reqwest::blocking::Client, String> {
    // rustls has no provider of its own when more than one is linked; pick one.
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|error| error.to_string())
}

/// `GET path` on `server`, with its session; blocking.
pub fn get(
    server: &ServerAddress,
    path: &str,
    timeout: Duration,
) -> Result<reqwest::blocking::Response, String> {
    let mut request = client(timeout)?.get(server.url(path));
    if let Some(session) = server.bearer() {
        request = request.bearer_auth(session);
    }
    let response = request.send().map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "Linker-Edit answered {}.",
            response.status().as_u16()
        ));
    }
    Ok(response)
}
