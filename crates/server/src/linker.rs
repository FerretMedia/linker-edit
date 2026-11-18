// Signing in with a Linker account (linker.cool), which is optional: nothing in Linker-Edit needs it yet.
//
// Linker's sign-in is Clerk's, and Clerk is an OAuth server, the same one Linker's AI connector uses. So this is
// OAuth's authorization code flow with PKCE, for an app on the user's own computer (RFC 8252): the browser signs in
// at Linker and comes back to this server on 127.0.0.1, which swaps the code for tokens.
//
// Clerk has to know the app asking. Linker serves a Client ID Metadata Document for Linker-Edit at
// app.linker.cool/oauth/linker-edit.json, which Clerk reads (its client_id is that address). Until that is live, or
// when this server's address isn't in it, Linker-Edit registers itself once (Dynamic Client Registration) if Clerk
// allows it. `linkerClientId` in the config overrides both.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use linker_edit_protocol::LinkerAccount;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use crate::config::LinkerConfig;
use crate::net::{
    Clock, Fetch, HttpFetch, Listener, Listeners, Request, Response, base64url, millis,
    random_bytes, sha256, system_clock, text_field, write_private,
};

pub struct LinkerOptions {
    /// Where the sign-in is kept, readable only by this user.
    pub file: PathBuf,
    /// Clerk's Frontend API, which is Linker's OAuth server.
    pub auth_server: String,
    /// Linker's dashboard and API.
    pub app_url: String,
    /// A client id to use instead of the metadata document or registering.
    pub client_id: Option<String>,
    /// This server's callback, on 127.0.0.1: http://127.0.0.1:<port>/api/linker/callback.
    pub redirect_uri: String,
    pub fetch: Arc<dyn Fetch>,
    pub now: Clock,
}

impl LinkerOptions {
    /// From the config, with the real network and clock.
    pub fn new(config: &LinkerConfig, redirect_uri: impl Into<String>) -> Self {
        LinkerOptions {
            file: config.file.clone(),
            auth_server: config.auth_server.clone(),
            app_url: config.app_url.clone(),
            client_id: config.client_id.clone(),
            redirect_uri: redirect_uri.into(),
            fetch: Arc::new(HttpFetch::default()),
            now: system_clock(),
        }
    }
}

/// What Linker-Edit asks to see: who the person is, and to stay signed in.
pub const SCOPES: &str = "openid profile email offline_access";

const SIGN_IN_TIMEOUT: i64 = 10 * 60_000;
const NETWORK_TIMEOUT: Duration = Duration::from_secs(15);
/// Refresh this long before the access token runs out.
const EXPIRY_MARGIN: i64 = 60_000;
const RAN_OUT: &str = "Your Linker sign-in ran out. Sign in again.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Client {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    secret: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Registered {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    secret: Option<String>,
    auth_server: String,
    redirect_uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    auth_server: String,
    client: Client,
    access_token: String,
    refresh_token: Option<String>,
    /// Milliseconds since 1970.
    #[serde(deserialize_with = "millis")]
    expires_at: i64,
    /// Linker's id for the person (the token's subject); missing in sign-ins kept before Linker-Edit needed it.
    #[serde(default)]
    user_id: Option<String>,
    #[serde(flatten)]
    profile: Profile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Profile {
    name: String,
    email: Option<String>,
    picture: Option<String>,
    plan: Option<String>,
}

/// ~/LinkerEdit/linker-account.json.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Saved {
    /// A client Linker-Edit registered itself, kept after signing out so it isn't registered again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    registered: Option<Registered>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session: Option<Session>,
}

#[derive(Debug, Clone, Deserialize)]
struct ServerMetadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    revocation_endpoint: Option<String>,
    registration_endpoint: Option<String>,
    userinfo_endpoint: Option<String>,
    client_id_metadata_document_supported: Option<bool>,
}

struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<f64>,
}

/// A refusal from Linker's OAuth server, as opposed to not reaching it.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct OAuthError {
    pub code: String,
    pub message: String,
}

/// A PKCE verifier and its S256 challenge.
pub fn pkce() -> (String, String) {
    let verifier = base64url(&random_bytes::<32>());
    let challenge = base64url(&sha256(&verifier));
    (verifier, challenge)
}

/// The metadata document Linker serves for Linker-Edit, which is also its client id.
pub fn metadata_document_url(app_url: &str) -> String {
    format!("{}/oauth/linker-edit.json", app_url.trim_end_matches('/'))
}

struct Pending {
    state: String,
    verifier: String,
    client: Client,
    started_at: i64,
}

struct State {
    saved: Saved,
    pending: Option<Pending>,
    problem: Option<String>,
    metadata: Option<ServerMetadata>,
}

/// The sign-in with Linker, kept in LinkerOptions::file.
pub struct Linker {
    file: PathBuf,
    auth_server: String,
    app_url: String,
    client_id: Option<String>,
    redirect_uri: String,
    fetch: Arc<dyn Fetch>,
    now: Clock,
    state: Mutex<State>,
    listeners: Listeners,
}

/// String(value) of a JSON field that isn't a string, for messages.
fn shown(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

impl Linker {
    pub fn new(options: LinkerOptions) -> Self {
        let saved = std::fs::read_to_string(&options.file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Linker {
            file: options.file,
            auth_server: options.auth_server.trim_end_matches('/').to_string(),
            app_url: options.app_url.trim_end_matches('/').to_string(),
            client_id: options.client_id,
            redirect_uri: options.redirect_uri,
            fetch: options.fetch,
            now: options.now,
            state: Mutex::new(State {
                saved,
                pending: None,
                problem: None,
                metadata: None,
            }),
            listeners: Listeners::default(),
        }
    }

    fn now(&self) -> i64 {
        (self.now)()
    }

    fn write(&self, saved: &Saved) -> anyhow::Result<()> {
        write_private(&self.file, &crate::js::stringify_pretty(saved))?;
        Ok(())
    }

    /// Writes what's kept now; a failure is logged, as nothing waits on it.
    fn save(&self) {
        let saved = self.state.lock().unwrap().saved.clone();
        if let Err(error) = self.write(&saved) {
            tracing::error!("Couldn't keep the Linker sign-in: {error:#}");
        }
    }

    pub fn status(&self) -> LinkerAccount {
        let state = self.state.lock().unwrap();
        if let Some(session) = &state.saved.session
            && session.auth_server == self.auth_server
        {
            return LinkerAccount::SignedIn {
                name: session.profile.name.clone(),
                email: session.profile.email.clone(),
                picture: session.profile.picture.clone(),
                plan: session.profile.plan.clone(),
                app_url: self.app_url.clone(),
            };
        }
        if let Some(pending) = &state.pending
            && self.now() - pending.started_at < SIGN_IN_TIMEOUT
        {
            return LinkerAccount::SigningIn;
        }
        LinkerAccount::SignedOut {
            problem: state.problem.clone(),
        }
    }

    pub fn on_status(&self, listener: Listener) {
        self.listeners.add(listener);
    }

    async fn call(&self, request: Request) -> anyhow::Result<Response> {
        self.fetch.fetch(request.timeout(NETWORK_TIMEOUT)).await
    }

    async fn server_metadata(&self) -> anyhow::Result<ServerMetadata> {
        if let Some(metadata) = self.state.lock().unwrap().metadata.clone() {
            return Ok(metadata);
        }
        let res = self
            .call(
                Request::get(format!(
                    "{}/.well-known/oauth-authorization-server",
                    self.auth_server
                ))
                .header("accept", "application/json"),
            )
            .await?;
        if !res.ok() {
            return Err(anyhow!("Linker's sign-in answered {}.", res.status));
        }
        let metadata: ServerMetadata = serde_json::from_slice(&res.body)?;
        self.state.lock().unwrap().metadata = Some(metadata.clone());
        Ok(metadata)
    }

    /// Whether Linker's metadata document for Linker-Edit is live, and the callbacks it lists when it is.
    async fn document_callbacks(&self, url: &str) -> Option<Vec<String>> {
        let res = self
            .call(Request::get(url).header("accept", "application/json"))
            .await
            .ok()?;
        if !res.ok() {
            return None;
        }
        let doc = res.json_value()?;
        if doc.get("client_id").and_then(Value::as_str) != Some(url) {
            return None;
        }
        let uris = doc.get("redirect_uris")?.as_array()?;
        Some(
            uris.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect(),
        )
    }

    async fn register(
        &self,
        meta: &ServerMetadata,
        callbacks: Option<Vec<String>>,
    ) -> anyhow::Result<Client> {
        if let Some(known) = &self.state.lock().unwrap().saved.registered
            && known.auth_server == self.auth_server
            && known.redirect_uri == self.redirect_uri
        {
            return Ok(Client {
                id: known.id.clone(),
                secret: known.secret.clone(),
            });
        }
        let endpoint = meta
            .registration_endpoint
            .clone()
            .unwrap_or_else(|| format!("{}/oauth/register", self.auth_server));
        let body = json!({
            "client_name": "Linker-Edit",
            "client_uri": "https://github.com/FerretMedia/linker-edit",
            "redirect_uris": [self.redirect_uri],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
            "scope": SCOPES,
        });
        let res = self
            .call(
                Request::new("POST", endpoint)
                    .header("content-type", "application/json")
                    .header("accept", "application/json")
                    .body(body.to_string()),
            )
            .await?;
        let answer = res.json_value().unwrap_or(Value::Null);
        let client_id = text_field(&answer, "client_id");
        let Some(client_id) = client_id.filter(|_| res.ok()) else {
            // Linker knows Linker-Edit, only not at this address: the port was changed in the config.
            let port = callbacks.iter().flatten().find_map(|uri| {
                Url::parse(uri)
                    .ok()
                    .and_then(|url| url.port())
                    .map(|port| port.to_string())
            });
            return Err(anyhow!(match port {
                Some(port) => format!(
                    "Linker only lets Linker-Edit sign in on port {port}. Take \"port\" out of linker-edit.config.json to sign in."
                ),
                None => "Linker doesn't accept Linker-Edit's sign-in yet. Try again after Linker's next release.".to_string(),
            }));
        };
        let registered = Registered {
            id: client_id,
            secret: text_field(&answer, "client_secret"),
            auth_server: self.auth_server.clone(),
            redirect_uri: self.redirect_uri.clone(),
        };
        let saved = {
            let mut state = self.state.lock().unwrap();
            state.saved.registered = Some(registered.clone());
            state.saved.clone()
        };
        self.write(&saved)?;
        Ok(Client {
            id: registered.id,
            secret: registered.secret,
        })
    }

    async fn client(&self, meta: &ServerMetadata) -> anyhow::Result<Client> {
        if let Some(id) = &self.client_id {
            return Ok(Client {
                id: id.clone(),
                secret: None,
            });
        }
        let document = metadata_document_url(&self.app_url);
        let callbacks = if meta.client_id_metadata_document_supported == Some(true) {
            self.document_callbacks(&document).await
        } else {
            None
        };
        if callbacks
            .as_ref()
            .is_some_and(|uris| uris.contains(&self.redirect_uri))
        {
            return Ok(Client {
                id: document,
                secret: None,
            });
        }
        self.register(meta, callbacks).await
    }

    fn client_fields(client: &Client) -> Vec<(&'static str, String)> {
        let mut fields = vec![("client_id", client.id.clone())];
        if let Some(secret) = &client.secret {
            fields.push(("client_secret", secret.clone()));
        }
        fields
    }

    async fn token_request(
        &self,
        meta: &ServerMetadata,
        fields: Vec<(&'static str, String)>,
    ) -> anyhow::Result<Tokens> {
        let form = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(fields)
            .finish();
        let res = self
            .call(
                Request::new("POST", meta.token_endpoint.clone())
                    .header("content-type", "application/x-www-form-urlencoded")
                    .header("accept", "application/json")
                    .body(form),
            )
            .await?;
        let body = res.json_value().unwrap_or(Value::Null);
        let access_token = text_field(&body, "access_token");
        match access_token.filter(|_| res.ok()) {
            Some(access_token) => Ok(Tokens {
                access_token,
                refresh_token: body
                    .get("refresh_token")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                expires_in: body.get("expires_in").and_then(Value::as_f64),
            }),
            None => Err(OAuthError {
                code: shown(body.get("error")).unwrap_or_else(|| format!("http_{}", res.status)),
                message: shown(body.get("error_description"))
                    .unwrap_or_else(|| format!("Linker answered {}.", res.status)),
            }
            .into()),
        }
    }

    /// Who the token belongs to, and the plan of the organization their AI apps work on.
    async fn profile(
        &self,
        meta: &ServerMetadata,
        access_token: &str,
    ) -> anyhow::Result<(Option<String>, Profile)> {
        let auth = |request: Request| {
            request
                .header("authorization", format!("Bearer {access_token}"))
                .header("accept", "application/json")
        };
        let userinfo = meta
            .userinfo_endpoint
            .clone()
            .unwrap_or_else(|| format!("{}/oauth/userinfo", self.auth_server));
        let res = self.call(auth(Request::get(userinfo))).await?;
        if !res.ok() {
            return Err(anyhow!("Linker didn't say who you are ({}).", res.status));
        }
        let info: Value = serde_json::from_slice(&res.body)?;
        let text = |key: &str| text_field(&info, key);
        let full_name = [text("given_name"), text("family_name")]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        let email = text("email");
        // The plan is a nice-to-have: a 403 means the person switched the AI connector off in Linker.
        let mut plan = None;
        if let Ok(account) = self
            .call(auth(Request::get(format!(
                "{}/api/connector",
                self.app_url
            ))))
            .await
            && account.ok()
        {
            // Offline or Linker is busy; the name is enough.
            plan = account
                .json_value()
                .and_then(|body| text_field(&body, "plan"));
        }
        let name = text("name")
            .or((!full_name.is_empty()).then_some(full_name))
            .or(text("preferred_username"))
            .or(email.clone())
            .unwrap_or_else(|| "Linker account".to_string());
        Ok((
            text("sub"),
            Profile {
                name,
                email,
                picture: text("picture"),
                plan,
            },
        ))
    }

    fn signed_out(&self, reason: Option<String>) {
        {
            let mut state = self.state.lock().unwrap();
            state.saved.session = None;
            state.problem = reason;
        }
        self.save();
        self.listeners.notify();
    }

    /// A token that works for a while yet, refreshed when needed; None when signed out. Fails when Linker can't be
    /// reached (still signed in) or refuses the refresh (signed out).
    pub async fn access_token(&self) -> anyhow::Result<Option<String>> {
        let session = {
            let state = self.state.lock().unwrap();
            match &state.saved.session {
                Some(session) if session.auth_server == self.auth_server => session.clone(),
                _ => return Ok(None),
            }
        };
        if session.expires_at - EXPIRY_MARGIN > self.now() {
            return Ok(Some(session.access_token));
        }
        let Some(refresh_token) = session.refresh_token.clone() else {
            self.signed_out(Some(RAN_OUT.into()));
            return Ok(None);
        };
        let meta = self.server_metadata().await?;
        let mut fields = vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token),
        ];
        fields.extend(Self::client_fields(&session.client));
        match self.token_request(&meta, fields).await {
            Ok(tokens) => {
                let saved = {
                    let mut state = self.state.lock().unwrap();
                    let now = self.now();
                    if let Some(kept) = state.saved.session.as_mut() {
                        kept.access_token = tokens.access_token.clone();
                        if let Some(refresh) = tokens.refresh_token {
                            kept.refresh_token = Some(refresh);
                        }
                        kept.expires_at =
                            now + (tokens.expires_in.unwrap_or(3600.0) * 1000.0) as i64;
                    }
                    state.saved.clone()
                };
                self.write(&saved)?;
                Ok(Some(tokens.access_token))
            }
            Err(error) => {
                // Only Linker saying no ends the sign-in; being offline doesn't.
                if error
                    .downcast_ref::<OAuthError>()
                    .is_some_and(|oauth| oauth.code == "invalid_grant")
                {
                    self.signed_out(Some(RAN_OUT.into()));
                }
                Err(error)
            }
        }
    }

    /// Starts signing in; the browser goes to the address this returns.
    pub async fn start(&self) -> anyhow::Result<String> {
        let meta = self.server_metadata().await?;
        let chosen = self.client(&meta).await?;
        let (verifier, challenge) = pkce();
        let state = base64url(&random_bytes::<16>());
        {
            let mut current = self.state.lock().unwrap();
            current.pending = Some(Pending {
                state: state.clone(),
                verifier,
                client: chosen.clone(),
                started_at: self.now(),
            });
            current.problem = None;
        }
        self.listeners.notify();
        let mut url = Url::parse(&meta.authorization_endpoint)?;
        url.query_pairs_mut()
            .clear()
            .append_pair("response_type", "code")
            .append_pair("client_id", &chosen.id)
            .append_pair("redirect_uri", &self.redirect_uri)
            .append_pair("scope", SCOPES)
            .append_pair("state", &state)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");
        Ok(url.to_string())
    }

    /// The browser's return from Linker (/api/linker/callback's query). Answers with the name signed in as, or
    /// fails with why not, worded for the user.
    pub async fn finish(&self, query: &HashMap<String, String>) -> anyhow::Result<String> {
        let get = |key: &str| query.get(key).cloned();
        let started = {
            let mut state = self.state.lock().unwrap();
            let fits = state.pending.as_ref().is_some_and(|pending| {
                get("state").as_deref() == Some(pending.state.as_str())
                    && self.now() - pending.started_at <= SIGN_IN_TIMEOUT
            });
            if !fits {
                return Err(anyhow!(
                    "This sign-in isn't the one Linker-Edit started, or it took too long. Start again from Linker-Edit."
                ));
            }
            state.pending.take().expect("checked above")
        };
        let result = async {
            if let Some(denied) = get("error").filter(|denied| !denied.is_empty()) {
                return Err(anyhow!(if denied == "access_denied" {
                    "Signing in was cancelled.".to_string()
                } else {
                    get("error_description").unwrap_or(denied)
                }));
            }
            let meta = self.server_metadata().await?;
            if let Some(iss) = get("iss").filter(|iss| !iss.is_empty())
                && iss != meta.issuer
            {
                return Err(anyhow!(
                    "The sign-in came back from somewhere other than Linker."
                ));
            }
            let Some(code) = get("code").filter(|code| !code.is_empty()) else {
                return Err(anyhow!("Linker sent no sign-in code."));
            };
            let mut fields = vec![
                ("grant_type", "authorization_code".to_string()),
                ("code", code),
                ("redirect_uri", self.redirect_uri.clone()),
                ("code_verifier", started.verifier.clone()),
            ];
            fields.extend(Self::client_fields(&started.client));
            let tokens = self.token_request(&meta, fields).await?;
            let (user_id, profile) = self.profile(&meta, &tokens.access_token).await?;
            let name = profile.name.clone();
            let saved = {
                let mut state = self.state.lock().unwrap();
                state.saved.session = Some(Session {
                    auth_server: self.auth_server.clone(),
                    client: started.client.clone(),
                    access_token: tokens.access_token,
                    refresh_token: tokens.refresh_token,
                    expires_at: self.now() + (tokens.expires_in.unwrap_or(3600.0) * 1000.0) as i64,
                    user_id,
                    profile,
                });
                state.problem = None;
                state.saved.clone()
            };
            self.write(&saved)?;
            Ok(name)
        }
        .await;
        if let Err(error) = &result {
            self.state.lock().unwrap().problem = Some(error.to_string());
        }
        self.listeners.notify();
        result
    }

    pub fn cancel(&self) {
        self.state.lock().unwrap().pending = None;
        self.listeners.notify();
    }

    pub async fn sign_out(&self) {
        let session = {
            let mut state = self.state.lock().unwrap();
            state.pending = None;
            state.saved.session.clone()
        };
        self.signed_out(None);
        let Some(session) = session else { return };
        // Best effort: the tokens are gone from this computer either way. Offline: Linker lets the token run out.
        let Ok(meta) = self.server_metadata().await else {
            return;
        };
        let Some(endpoint) = meta.revocation_endpoint else {
            return;
        };
        let token = session
            .refresh_token
            .clone()
            .unwrap_or(session.access_token.clone());
        let mut fields = vec![("token", token)];
        fields.extend(Self::client_fields(&session.client));
        let form = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(fields)
            .finish();
        let _ = self
            .call(
                Request::new("POST", endpoint)
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(form),
            )
            .await;
    }

    /// Linker's id for the person signed in here; None when signed out or not known yet.
    pub fn user_id(&self) -> Option<String> {
        let state = self.state.lock().unwrap();
        state
            .saved
            .session
            .as_ref()
            .filter(|session| session.auth_server == self.auth_server)
            .and_then(|session| session.user_id.clone())
    }

    /// Who a Linker token belongs to, as Linker's id for them: None when Linker says the token isn't valid.
    /// Fails when Linker can't be reached, so being offline isn't taken for a wrong token.
    pub async fn who_is(&self, token: &str) -> anyhow::Result<Option<String>> {
        let meta = self.server_metadata().await?;
        let userinfo = meta
            .userinfo_endpoint
            .clone()
            .unwrap_or_else(|| format!("{}/oauth/userinfo", self.auth_server));
        let res = self
            .call(
                Request::get(userinfo)
                    .header("authorization", format!("Bearer {token}"))
                    .header("accept", "application/json"),
            )
            .await?;
        if res.status == 401 || res.status == 403 {
            return Ok(None);
        }
        if !res.ok() {
            return Err(anyhow!("Linker didn't say who that is ({}).", res.status));
        }
        let info: Value = serde_json::from_slice(&res.body)?;
        Ok(text_field(&info, "sub"))
    }

    /// Brings the name and plan up to date, at start. Offline keeps what was there.
    pub async fn refresh(&self) {
        let updated: anyhow::Result<bool> = async {
            let Some(token) = self.access_token().await? else {
                return Ok(false);
            };
            let meta = self.server_metadata().await?;
            let (user_id, profile) = self.profile(&meta, &token).await?;
            let saved = {
                let mut state = self.state.lock().unwrap();
                let Some(session) = state.saved.session.as_mut() else {
                    return Ok(false);
                };
                session.user_id = user_id;
                session.profile = profile;
                state.saved.clone()
            };
            self.write(&saved)?;
            Ok(true)
        }
        .await;
        // Offline or Linker is busy: still signed in, with what was known.
        if let Ok(true) = updated {
            self.listeners.notify();
        }
    }
}

/// What Linker Connect needs of the Linker sign-in; Linker is the real one, tests use a stand-in.
#[async_trait]
pub trait LinkerService: Send + Sync {
    fn status(&self) -> LinkerAccount;
    fn on_status(&self, listener: Listener);
    async fn access_token(&self) -> anyhow::Result<Option<String>>;
    fn user_id(&self) -> Option<String>;
    async fn refresh(&self);
    async fn who_is(&self, token: &str) -> anyhow::Result<Option<String>>;
}

#[async_trait]
impl LinkerService for Linker {
    fn status(&self) -> LinkerAccount {
        Linker::status(self)
    }
    fn on_status(&self, listener: Listener) {
        Linker::on_status(self, listener)
    }
    async fn access_token(&self) -> anyhow::Result<Option<String>> {
        Linker::access_token(self).await
    }
    fn user_id(&self) -> Option<String> {
        Linker::user_id(self)
    }
    async fn refresh(&self) {
        Linker::refresh(self).await
    }
    async fn who_is(&self, token: &str) -> anyhow::Result<Option<String>> {
        Linker::who_is(self, token).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::hex;
    use std::sync::atomic::{AtomicI64, Ordering};

    const AUTH: &str = "https://clerk.example";
    const APP: &str = "https://app.example";
    const REDIRECT: &str = "http://127.0.0.1:4317/api/linker/callback";

    fn document() -> String {
        metadata_document_url(APP)
    }

    #[derive(Default, Clone)]
    struct FakeOptions {
        cimd: Option<bool>,
        /// Some(None): the document isn't live.
        document_lists: Option<Option<Vec<String>>>,
        registration: Option<bool>,
        plan: Option<u16>,
        refresh: Option<&'static str>,
    }

    /// Clerk and Linker's API as far as signing in goes; records what was asked.
    #[derive(Default)]
    struct FakeLinker {
        options: FakeOptions,
        calls: Mutex<Vec<(String, String)>>,
        issued: Mutex<u32>,
        challenge: Mutex<String>,
    }

    fn form(body: &str) -> HashMap<String, String> {
        url::form_urlencoded::parse(body.as_bytes())
            .into_owned()
            .collect()
    }

    impl FakeLinker {
        fn new(options: FakeOptions) -> Arc<Self> {
            Arc::new(FakeLinker {
                options,
                ..Default::default()
            })
        }

        /// What Clerk keeps from the authorize request.
        fn authorize(&self, location: &str) -> HashMap<String, String> {
            let params: HashMap<String, String> = Url::parse(location)
                .unwrap()
                .query_pairs()
                .into_owned()
                .collect();
            *self.challenge.lock().unwrap() =
                params.get("code_challenge").cloned().unwrap_or_default();
            params
        }

        fn calls(&self) -> Vec<(String, String)> {
            self.calls.lock().unwrap().clone()
        }

        fn issue(&self, refresh: &str) -> Response {
            let mut issued = self.issued.lock().unwrap();
            *issued += 1;
            Response::json(
                200,
                &json!({ "access_token": format!("access-{issued}"), "refresh_token": refresh, "expires_in": 3600 }),
            )
        }
    }

    #[async_trait]
    impl Fetch for FakeLinker {
        async fn fetch(&self, request: Request) -> anyhow::Result<Response> {
            let url = request.url.clone();
            let body = request.body.clone().unwrap_or_default();
            self.calls.lock().unwrap().push((url.clone(), body.clone()));
            let fake = &self.options;
            if url == format!("{AUTH}/.well-known/oauth-authorization-server") {
                return Ok(Response::json(
                    200,
                    &json!({
                        "issuer": AUTH,
                        "authorization_endpoint": format!("{AUTH}/oauth/authorize"),
                        "token_endpoint": format!("{AUTH}/oauth/token"),
                        "revocation_endpoint": format!("{AUTH}/oauth/token/revoke"),
                        "client_id_metadata_document_supported": fake.cimd.unwrap_or(true),
                    }),
                ));
            }
            if url == document() {
                return Ok(match &fake.document_lists {
                    Some(None) => Response::new(404, "Not found"),
                    Some(Some(list)) => Response::json(
                        200,
                        &json!({ "client_id": document(), "redirect_uris": list }),
                    ),
                    None => Response::json(
                        200,
                        &json!({ "client_id": document(), "redirect_uris": [REDIRECT] }),
                    ),
                });
            }
            if url == format!("{AUTH}/oauth/register") {
                return Ok(if fake.registration == Some(false) {
                    Response::json(403, &json!({ "error": "registration_disabled" }))
                } else {
                    Response::json(201, &json!({ "client_id": "registered-client" }))
                });
            }
            if url == format!("{AUTH}/oauth/token") {
                let form = form(&body);
                if form.get("grant_type").map(String::as_str) == Some("authorization_code") {
                    let verifier = form.get("code_verifier").cloned().unwrap_or_default();
                    if base64url(&sha256(&verifier)) != *self.challenge.lock().unwrap() {
                        return Ok(Response::json(400, &json!({ "error": "invalid_grant" })));
                    }
                    return Ok(self.issue("refresh-1"));
                }
                return match fake.refresh {
                    Some("offline") => Err(anyhow!("fetch failed")),
                    Some("invalid_grant") => {
                        Ok(Response::json(400, &json!({ "error": "invalid_grant" })))
                    }
                    _ => Ok(self.issue("refresh-2")),
                };
            }
            if url == format!("{AUTH}/oauth/userinfo") {
                return Ok(match request.header_value("authorization") {
                    Some("Bearer stolen") => {
                        Response::json(401, &json!({ "error": "invalid_token" }))
                    }
                    Some("Bearer bob") => {
                        Response::json(200, &json!({ "sub": "user_bob", "name": "Bob" }))
                    }
                    _ => Response::json(
                        200,
                        &json!({ "sub": "user_jane", "name": "Jane Doe", "email": "jane@example.com", "picture": "https://img.example/jane.png" }),
                    ),
                });
            }
            if url == format!("{APP}/api/connector") {
                return Ok(Response::json(
                    fake.plan.unwrap_or(200),
                    &json!({ "plan": "Max", "paid": true }),
                ));
            }
            if url == format!("{AUTH}/oauth/token/revoke") {
                return Ok(Response::new(200, ""));
            }
            Ok(Response::new(404, "Not found"))
        }
    }

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
            self.dir.path().join("linker-account.json")
        }

        fn linker(&self, fake: &Arc<FakeLinker>) -> Linker {
            self.linker_with(fake, None)
        }

        fn linker_with(&self, fake: &Arc<FakeLinker>, client_id: Option<&str>) -> Linker {
            let clock = self.clock.clone();
            Linker::new(LinkerOptions {
                file: self.file(),
                auth_server: AUTH.into(),
                app_url: APP.into(),
                client_id: client_id.map(str::to_string),
                redirect_uri: REDIRECT.into(),
                fetch: fake.clone(),
                now: Arc::new(move || clock.load(Ordering::SeqCst)),
            })
        }

        fn advance(&self, ms: i64) {
            self.clock.fetch_add(ms, Ordering::SeqCst);
        }
    }

    fn query(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    async fn sign_in(linker: &Linker, fake: &FakeLinker) -> anyhow::Result<String> {
        let params = fake.authorize(&linker.start().await?);
        linker
            .finish(&query(&[
                ("code", "the-code"),
                ("state", &params["state"]),
                ("iss", AUTH),
            ]))
            .await
    }

    fn signed_out(problem: Option<&str>) -> LinkerAccount {
        LinkerAccount::SignedOut {
            problem: problem.map(str::to_string),
        }
    }

    fn signed_in_name(status: LinkerAccount) -> (String, Option<String>) {
        match status {
            LinkerAccount::SignedIn { name, plan, .. } => (name, plan),
            other => panic!("not signed in: {other:?}"),
        }
    }

    #[test]
    fn pkce_makes_an_s256_challenge_of_the_verifier() {
        let (verifier, challenge) = pkce();
        assert_eq!(verifier.len(), 43);
        assert!(
            verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        );
        assert_eq!(challenge, base64url(&sha256(&verifier)));
        assert_eq!(hex(&sha256("")).len(), 64);
    }

    #[tokio::test]
    async fn keeps_the_accounts_id_and_checks_other_tokens_with_linker() {
        let setup = Setup::new();
        let fake = FakeLinker::new(FakeOptions::default());
        let linker = setup.linker(&fake);
        assert_eq!(linker.user_id(), None);
        sign_in(&linker, &fake).await.unwrap();
        assert_eq!(linker.user_id().as_deref(), Some("user_jane"));
        assert_eq!(
            linker.who_is("bob").await.unwrap().as_deref(),
            Some("user_bob")
        );
        assert_eq!(linker.who_is("stolen").await.unwrap(), None);
        // A sign-in kept by an older Linker-Edit gets its id when it refreshes at start.
        let mut saved: Value =
            serde_json::from_str(&std::fs::read_to_string(setup.file()).unwrap()).unwrap();
        saved["session"].as_object_mut().unwrap().remove("userId");
        std::fs::write(setup.file(), saved.to_string()).unwrap();
        let restarted = setup.linker(&fake);
        assert_eq!(restarted.user_id(), None);
        restarted.refresh().await;
        assert_eq!(restarted.user_id().as_deref(), Some("user_jane"));
    }

    #[test]
    fn starts_signed_out_and_isnt_needed_for_anything() {
        let setup = Setup::new();
        let linker = setup.linker(&FakeLinker::new(FakeOptions::default()));
        assert_eq!(linker.status(), signed_out(None));
    }

    #[tokio::test]
    async fn signs_in_with_linkers_metadata_document_as_the_client() {
        let setup = Setup::new();
        let fake = FakeLinker::new(FakeOptions::default());
        let linker = setup.linker(&fake);
        let location = linker.start().await.unwrap();
        assert_eq!(linker.status(), LinkerAccount::SigningIn);
        let params = fake.authorize(&location);
        assert!(location.starts_with(&format!("{AUTH}/oauth/authorize?")));
        assert_eq!(params["response_type"], "code");
        assert_eq!(params["client_id"], document());
        assert_eq!(params["redirect_uri"], REDIRECT);
        assert_eq!(params["scope"], "openid profile email offline_access");
        assert_eq!(params["code_challenge_method"], "S256");

        let name = linker
            .finish(&query(&[
                ("code", "the-code"),
                ("state", &params["state"]),
                ("iss", AUTH),
            ]))
            .await
            .unwrap();
        assert_eq!(name, "Jane Doe");
        assert_eq!(
            linker.status(),
            LinkerAccount::SignedIn {
                name: "Jane Doe".into(),
                email: Some("jane@example.com".into()),
                picture: Some("https://img.example/jane.png".into()),
                plan: Some("Max".into()),
                app_url: APP.into(),
            }
        );
        assert!(
            !fake
                .calls()
                .iter()
                .any(|(url, _)| url.ends_with("/oauth/register"))
        );
    }

    #[tokio::test]
    async fn keeps_the_sign_in_for_next_time_readable_by_this_user_only() {
        let setup = Setup::new();
        let fake = FakeLinker::new(FakeOptions::default());
        sign_in(&setup.linker(&fake), &fake).await.unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(setup.file())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0);
        }
        assert_eq!(signed_in_name(setup.linker(&fake).status()).0, "Jane Doe");
    }

    #[tokio::test]
    async fn reads_a_sign_in_the_node_app_kept() {
        let setup = Setup::new();
        std::fs::write(
            setup.file(),
            r#"{
  "registered": { "id": "registered-client", "authServer": "https://clerk.example", "redirectUri": "http://127.0.0.1:4317/api/linker/callback" },
  "session": {
    "authServer": "https://clerk.example",
    "client": { "id": "https://app.example/oauth/linker-edit.json" },
    "accessToken": "access-1",
    "refreshToken": "refresh-1",
    "expiresAt": 4600000,
    "userId": "user_jane",
    "name": "Jane Doe",
    "email": "jane@example.com",
    "picture": null,
    "plan": "Max"
  }
}"#,
        )
        .unwrap();
        let linker = setup.linker(&FakeLinker::new(FakeOptions::default()));
        assert_eq!(
            signed_in_name(linker.status()),
            ("Jane Doe".into(), Some("Max".into()))
        );
        assert_eq!(
            linker.access_token().await.unwrap().as_deref(),
            Some("access-1")
        );
        // Written back the same way.
        linker.save();
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(setup.file()).unwrap()).unwrap();
        assert_eq!(written["session"]["expiresAt"], json!(4600000));
        assert_eq!(
            written["session"]["client"],
            json!({ "id": "https://app.example/oauth/linker-edit.json" })
        );
        assert_eq!(written["registered"]["redirectUri"], json!(REDIRECT));
    }

    #[tokio::test]
    async fn registers_itself_once_while_the_metadata_document_isnt_live() {
        let setup = Setup::new();
        let fake = FakeLinker::new(FakeOptions {
            document_lists: Some(None),
            ..Default::default()
        });
        let linker = setup.linker(&fake);
        assert_eq!(
            fake.authorize(&linker.start().await.unwrap())["client_id"],
            "registered-client"
        );
        linker.start().await.unwrap();
        let registrations: Vec<_> = fake
            .calls()
            .into_iter()
            .filter(|(url, _)| url.ends_with("/oauth/register"))
            .collect();
        assert_eq!(registrations.len(), 1);
        let registration: Value = serde_json::from_str(&registrations[0].1).unwrap();
        assert_eq!(registration["redirect_uris"], json!([REDIRECT]));
        assert_eq!(registration["token_endpoint_auth_method"], "none");
    }

    #[tokio::test]
    async fn registers_when_the_metadata_document_doesnt_list_this_servers_port() {
        let setup = Setup::new();
        let fake = FakeLinker::new(FakeOptions {
            document_lists: Some(Some(vec![
                "http://127.0.0.1:9999/api/linker/callback".into(),
            ])),
            ..Default::default()
        });
        let location = setup.linker(&fake).start().await.unwrap();
        assert_eq!(fake.authorize(&location)["client_id"], "registered-client");
    }

    #[tokio::test]
    async fn uses_the_configured_client_id_over_both() {
        let setup = Setup::new();
        let fake = FakeLinker::new(FakeOptions::default());
        let location = setup
            .linker_with(&fake, Some("my-client"))
            .start()
            .await
            .unwrap();
        assert_eq!(fake.authorize(&location)["client_id"], "my-client");
    }

    #[tokio::test]
    async fn says_so_when_linker_accepts_neither() {
        let setup = Setup::new();
        let linker = setup.linker(&FakeLinker::new(FakeOptions {
            document_lists: Some(None),
            registration: Some(false),
            ..Default::default()
        }));
        let error = linker.start().await.unwrap_err().to_string();
        assert!(
            error.contains("doesn't accept Linker-Edit's sign-in yet"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn names_the_port_linker_allows_when_the_config_changed_it() {
        let setup = Setup::new();
        let fake = FakeLinker::new(FakeOptions {
            document_lists: Some(Some(vec![
                "http://127.0.0.1:4318/api/linker/callback".into(),
            ])),
            registration: Some(false),
            ..Default::default()
        });
        let error = setup.linker(&fake).start().await.unwrap_err().to_string();
        assert!(
            error.contains("only lets Linker-Edit sign in on port 4318"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn turns_away_a_return_it_didnt_start() {
        let setup = Setup::new();
        let fake = FakeLinker::new(FakeOptions::default());
        let linker = setup.linker(&fake);
        fake.authorize(&linker.start().await.unwrap());
        let error = linker
            .finish(&query(&[("code", "x"), ("state", "forged")]))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("isn't the one"), "{error}");
        // The real sign-in still works afterwards.
        assert_eq!(linker.status(), LinkerAccount::SigningIn);
    }

    #[tokio::test]
    async fn turns_away_a_return_from_another_issuer() {
        let setup = Setup::new();
        let fake = FakeLinker::new(FakeOptions::default());
        let linker = setup.linker(&fake);
        let params = fake.authorize(&linker.start().await.unwrap());
        let error = linker
            .finish(&query(&[
                ("code", "x"),
                ("state", &params["state"]),
                ("iss", "https://evil.example"),
            ]))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("somewhere other than Linker"), "{error}");
        assert_eq!(
            linker.status(),
            signed_out(Some(
                "The sign-in came back from somewhere other than Linker."
            ))
        );
    }

    #[tokio::test]
    async fn says_when_the_person_cancelled_at_linker() {
        let setup = Setup::new();
        let fake = FakeLinker::new(FakeOptions::default());
        let linker = setup.linker(&fake);
        let params = fake.authorize(&linker.start().await.unwrap());
        let error = linker
            .finish(&query(&[
                ("error", "access_denied"),
                ("state", &params["state"]),
            ]))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("cancelled"), "{error}");
    }

    #[tokio::test]
    async fn forgets_a_sign_in_left_waiting_too_long() {
        let setup = Setup::new();
        let fake = FakeLinker::new(FakeOptions::default());
        let linker = setup.linker(&fake);
        let params = fake.authorize(&linker.start().await.unwrap());
        setup.advance(11 * 60_000);
        assert_eq!(linker.status(), signed_out(None));
        let error = linker
            .finish(&query(&[("code", "x"), ("state", &params["state"])]))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("took too long"), "{error}");
    }

    #[tokio::test]
    async fn signs_in_without_a_plan_when_linkers_ai_connector_is_switched_off() {
        let setup = Setup::new();
        let fake = FakeLinker::new(FakeOptions {
            plan: Some(403),
            ..Default::default()
        });
        let linker = setup.linker(&fake);
        sign_in(&linker, &fake).await.unwrap();
        assert_eq!(signed_in_name(linker.status()).1, None);
    }

    #[tokio::test]
    async fn refreshes_a_token_that_ran_out() {
        let setup = Setup::new();
        let fake = FakeLinker::new(FakeOptions::default());
        let linker = setup.linker(&fake);
        sign_in(&linker, &fake).await.unwrap();
        assert_eq!(
            linker.access_token().await.unwrap().as_deref(),
            Some("access-1")
        );
        setup.advance(2 * 3_600_000);
        assert_eq!(
            linker.access_token().await.unwrap().as_deref(),
            Some("access-2")
        );
    }

    #[tokio::test]
    async fn signs_out_when_linker_refuses_the_refresh() {
        let setup = Setup::new();
        let fake = FakeLinker::new(FakeOptions {
            refresh: Some("invalid_grant"),
            ..Default::default()
        });
        let linker = setup.linker(&fake);
        sign_in(&linker, &fake).await.unwrap();
        setup.advance(2 * 3_600_000);
        assert!(linker.access_token().await.is_err());
        assert_eq!(
            linker.status(),
            signed_out(Some("Your Linker sign-in ran out. Sign in again."))
        );
    }

    #[tokio::test]
    async fn stays_signed_in_while_offline() {
        let setup = Setup::new();
        let fake = FakeLinker::new(FakeOptions {
            refresh: Some("offline"),
            ..Default::default()
        });
        let linker = setup.linker(&fake);
        sign_in(&linker, &fake).await.unwrap();
        setup.advance(2 * 3_600_000);
        linker.refresh().await;
        assert_eq!(signed_in_name(linker.status()).0, "Jane Doe");
    }

    #[tokio::test]
    async fn signs_out_revoking_the_token_and_forgetting_it_here() {
        let setup = Setup::new();
        let fake = FakeLinker::new(FakeOptions::default());
        let linker = setup.linker(&fake);
        sign_in(&linker, &fake).await.unwrap();
        linker.sign_out().await;
        assert_eq!(linker.status(), signed_out(None));
        assert_eq!(setup.linker(&fake).status(), signed_out(None));
        let calls = fake.calls();
        let revoke = calls
            .iter()
            .find(|(url, _)| url.ends_with("/revoke"))
            .unwrap();
        assert_eq!(form(&revoke.1)["token"], "refresh-1");
    }
}
