//! Pictures from the server: GPUI's `img("http://…")` fetches through the app's HTTP client,
//! which is a no-op until one is set. This one is reqwest on a small tokio runtime of its own
//! (GPUI's executor isn't tokio), sending the Linker Connect session as Bearer when the window
//! is on another computer, as the browser sends its cookie. It serves the library page's
//! thumbnails (/library/…) and the history's clip tiles (/media/…).

use std::sync::Arc;

use futures::FutureExt;
use futures::future::BoxFuture;
use gpui::http_client::{AsyncBody, HttpClient, Request, Response, Url, http::HeaderValue};

pub struct Client {
    runtime: tokio::runtime::Runtime,
    client: reqwest::Client,
    bearer: Option<String>,
}

impl Client {
    pub fn new(bearer: Option<String>) -> anyhow::Result<Arc<Self>> {
        // rustls has no provider of its own when more than one is linked; pick one.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("linker-edit-http")
            .enable_all()
            .build()?;
        Ok(Arc::new(Client {
            runtime,
            client: reqwest::Client::new(),
            bearer,
        }))
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
        let mut builder = self
            .client
            .request(request.method().clone(), request.uri().to_string());
        if let Some(session) = &self.bearer {
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
