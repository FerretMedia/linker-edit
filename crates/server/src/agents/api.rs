// How processes started for the agents, such as their MCP servers, reach the Linker-Edit server.

use std::collections::BTreeMap;

use anyhow::Context as _;
use serde_json::Value;

/// The env a child process needs to call the API (Api::from_env).
pub fn api_env(port: u16, token: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "LINKER_EDIT_API".to_string(),
            format!("http://127.0.0.1:{port}"),
        ),
        ("LINKER_EDIT_TOKEN".to_string(), token.to_string()),
    ])
}

/// The Linker-Edit server's /api, as a child process given api_env sees it.
#[derive(Debug, Clone)]
pub struct Api {
    base: String,
    token: String,
    client: reqwest::Client,
}

impl Api {
    pub fn new(base: impl Into<String>, token: impl Into<String>) -> Self {
        Api {
            base: base.into(),
            token: token.into(),
            // No timeout: a first transcription downloads the engine, and an approval waits for the user.
            // No proxy: the server is on this computer.
            client: reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("an HTTP client without TLS settings builds"),
        }
    }

    pub fn from_env() -> Self {
        Api::new(
            std::env::var("LINKER_EDIT_API").unwrap_or_default(),
            std::env::var("LINKER_EDIT_TOKEN").unwrap_or_default(),
        )
    }

    /// POSTs JSON to a route such as "/api/resolve-tool"; the status and the body's text.
    pub async fn post(&self, route: &str, body: &Value) -> anyhow::Result<(u16, String)> {
        let response = self
            .client
            .post(format!("{}{route}", self.base))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {}", self.token))
            .body(body.to_string())
            .send()
            .await?;
        let status = response.status().as_u16();
        let text = response.text().await.context("reading the answer")?;
        Ok((status, text))
    }
}
