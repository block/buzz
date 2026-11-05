//! NIP-FA client: how the CLI obtains a NIP-FI assertion, the way Buzz Desktop
//! does (docs/nips/NIP-FA.md).
//!
//! ```text
//! browser login ──► adapter session ──► /v1/identity/assertions ──► nip-fi+jwt (own key)
//!                         └──► agent grant ──► /v1/nip-fi/agent-assertion ──► nip-fi-agent+jwt
//! ```
//!
//! The session and grants live in one private file on this device. Every run
//! that reaches a NIP-FI relay mints one fresh assertion; none is stored.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use nostr::Keys;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::client::sign_nip98;
use crate::error::CliError;

/// The adapter's agent-grant dialect. Every other route is NIP-FA in both.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Protocol {
    /// NIP-FA as specified.
    NipFa,
    /// Builderlab's deployed adapter, as Buzz Desktop speaks it: its own token
    /// types, a `runtime` field and a `Nostr-Agent-Proof` header.
    Builderlab,
}

impl Protocol {
    fn subject_token_type(self) -> &'static str {
        match self {
            Self::NipFa => "urn:nostr:params:oauth:token-type:nip-fa-session",
            Self::Builderlab => "urn:builderlab:params:oauth:token-type:bb-session",
        }
    }
    fn grant_token_type(self) -> &'static str {
        match self {
            Self::NipFa => "urn:nostr:params:oauth:token-type:nip-fi-agent-grant",
            Self::Builderlab => "urn:builderlab:params:oauth:token-type:nip-fi-agent-grant",
        }
    }
    fn grant_proof_header(self) -> &'static str {
        match self {
            Self::NipFa => "Nostr-Authorization",
            Self::Builderlab => "Nostr-Agent-Proof",
        }
    }
}

// ── Stored credentials ───────────────────────────────────────────────────────

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session: Option<Session>,
    /// Keyed by agent pubkey.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    grants: BTreeMap<String, Grant>,
}

/// An adapter session. No `Debug`: it must never reach a log line.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Session {
    adapter: String,
    protocol: Protocol,
    token: String,
    expires_at: i64,
}

/// An agent grant and everything it is bound to.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Grant {
    adapter: String,
    protocol: Protocol,
    agent: String,
    owner: String,
    issuer: String,
    token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<i64>,
}

const STORE_FILE: &str = "federated-identity.json";
const MAX_STORE_BYTES: u64 = 1024 * 1024;

/// The private credential file, read and replaced under an exclusive lock.
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// `<config dir>/buzz-cli`, e.g. `~/Library/Application Support/buzz-cli`.
    pub fn default_location() -> Result<Self, CliError> {
        let dir = dirs::config_dir()
            .ok_or_else(|| CliError::Other("no config directory for credentials".into()))?;
        Ok(Self {
            dir: dir.join("buzz-cli"),
        })
    }

    #[cfg(test)]
    fn at(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn file(&self) -> PathBuf {
        self.dir.join(STORE_FILE)
    }

    fn io(&self, e: std::io::Error) -> CliError {
        CliError::Other(format!("credential store {}: {e}", self.dir.display()))
    }

    fn refuse(&self) -> CliError {
        CliError::Other(format!(
            "credential store {} is not a private regular file and directory; refusing to use it",
            self.dir.display()
        ))
    }

    /// Whether credentials may exist, without locking.
    pub fn has_credentials(&self) -> bool {
        std::fs::symlink_metadata(self.file()).is_ok()
    }

    fn ensure_dir(&self) -> Result<(), CliError> {
        match std::fs::symlink_metadata(&self.dir) {
            Ok(m) if !m.is_dir() => return Err(self.refuse()),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut builder = std::fs::DirBuilder::new();
                builder.recursive(true);
                #[cfg(unix)]
                std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
                builder.create(&self.dir).map_err(|e| self.io(e))?;
            }
            Err(e) => return Err(self.io(e)),
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| self.io(e))?;
        }
        Ok(())
    }

    fn lock(&self) -> Result<std::fs::File, CliError> {
        self.ensure_dir()?;
        let path = self.dir.join("federated-identity.lock");
        if std::fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file()) {
            return Err(self.refuse());
        }
        let file = private_options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| self.io(e))?;
        fs2::FileExt::lock_exclusive(&file).map_err(|e| self.io(e))?;
        Ok(file)
    }

    fn read_locked(&self) -> Result<State, CliError> {
        let path = self.file();
        match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(State::default()),
            Err(e) => return Err(self.io(e)),
            Ok(m) if !m.is_file() || m.len() > MAX_STORE_BYTES => return Err(self.refuse()),
            Ok(_) => {}
        }
        let bytes = std::fs::read(&path).map_err(|e| self.io(e))?;
        serde_json::from_slice(&bytes).map_err(|_| {
            CliError::Other(format!(
                "credential store {} is unreadable; run `buzz identity logout` and sign in again",
                path.display()
            ))
        })
    }

    /// Replace the file atomically: a fresh 0600 temporary file, then rename.
    /// An empty state removes the file.
    fn write_locked(&self, state: &State) -> Result<(), CliError> {
        let path = self.file();
        if state.session.is_none() && state.grants.is_empty() {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(self.io(e)),
                _ => Ok(()),
            };
        }
        let bytes = serde_json::to_vec(state).map_err(|e| CliError::Other(e.to_string()))?;
        let tmp = self
            .dir
            .join(format!(".{STORE_FILE}.{}.tmp", uuid::Uuid::new_v4()));
        let written = (|| {
            use std::io::Write;
            let mut file = private_options().create_new(true).write(true).open(&tmp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            std::fs::rename(&tmp, &path)
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        written.map_err(|e| self.io(e))
    }

    fn read(&self) -> Result<State, CliError> {
        let _lock = self.lock()?;
        self.read_locked()
    }

    /// Change the state under the lock; `f` returns whether it changed it.
    fn update<T>(
        &self,
        f: impl FnOnce(&mut State) -> Result<(T, bool), CliError>,
    ) -> Result<T, CliError> {
        let _lock = self.lock()?;
        let mut state = self.read_locked()?;
        let (value, changed) = f(&mut state)?;
        if changed {
            self.write_locked(&state)?;
        }
        Ok(value)
    }

    /// Clear the session only if it is still the one that ended.
    fn clear_session(&self, ended: &Session) -> Result<(), CliError> {
        self.update(|state| {
            let current = state.session.as_ref() == Some(ended);
            if current {
                state.session = None;
            }
            Ok(((), current))
        })
    }

    /// Delete everything on this device. Enrollments stay on the adapter.
    pub fn logout(&self) -> Result<(), CliError> {
        self.update(|state| {
            *state = State::default();
            Ok(((), true))
        })
    }
}

fn private_options() -> std::fs::OpenOptions {
    let mut options = std::fs::OpenOptions::new();
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
}

// ── Adapter HTTP ─────────────────────────────────────────────────────────────

const ADAPTER_DEADLINE: Duration = Duration::from_secs(30);
const MAX_ADAPTER_BODY: usize = 64 * 1024;
const RETRY_ATTEMPTS: u32 = 3;

/// How an adapter call ended, when it did not succeed.
#[derive(Debug)]
enum Denied {
    /// 401 `session_required` / `session_expired`: the session is gone.
    SessionEnded,
    /// 401 `invalid_grant` on the agent-assertion route: the grant is gone.
    GrantEnded,
    /// A refusal, redirect, or unusable success body. Credentials are kept and
    /// nothing is retried automatically.
    Refused(String),
    /// Network, 429 or 5xx after bounded retries. Credentials are kept.
    Unavailable,
    /// The request could not be built (e.g. signing failed).
    Local(CliError),
}

impl Denied {
    fn into_error(self, what: &str) -> CliError {
        match self {
            Self::SessionEnded => CliError::Auth(format!(
                "{what}: the sign-in session ended; run `buzz identity login` again"
            )),
            Self::GrantEnded => CliError::Auth(format!(
                "{what}: the agent's grant was revoked or expired; run `buzz identity enroll-agent` again"
            )),
            Self::Refused(code) => CliError::Auth(format!("{what}: refused ({code})")),
            Self::Unavailable => CliError::Other(format!(
                "{what}: the identity service is unavailable; try again"
            )),
            Self::Local(e) => e,
        }
    }
}

fn adapter_client() -> Result<reqwest::Client, CliError> {
    // NIP-FA: no adapter call follows a redirect, so no credential is replayed.
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(ADAPTER_DEADLINE)
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| CliError::Other(e.to_string()))
}

/// Only a short, plain error code is ever repeated back to the user.
fn denial_code(body: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error")?.as_str().map(str::to_owned))
        .filter(|c| c.len() <= 64 && c.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
        .unwrap_or_else(|| "unknown".into())
}

async fn bounded_body(mut response: reqwest::Response) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        if chunk.len() > MAX_ADAPTER_BODY - body.len() {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    Some(body)
}

/// Send one adapter request, built afresh (with a fresh proof) per attempt.
/// Network failures, 429 and 5xx retry with backoff up to `attempts` times.
async fn adapter_json<T: DeserializeOwned>(
    attempts: u32,
    request: impl Fn() -> Result<reqwest::RequestBuilder, CliError>,
) -> Result<T, Denied> {
    for attempt in 0..attempts {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(250 << attempt)).await;
        }
        let response = match request().map_err(Denied::Local)?.send().await {
            Ok(response) => response,
            Err(_) => continue,
        };
        let status = response.status();
        if status.as_u16() == 429 || status.is_server_error() {
            continue;
        }
        if status.is_redirection() {
            return Err(Denied::Refused("redirect".into()));
        }
        let Some(body) = bounded_body(response).await else {
            return Err(Denied::Refused("response_too_large".into()));
        };
        if status.is_success() {
            return serde_json::from_slice(&body)
                .map_err(|_| Denied::Refused("invalid_response".into()));
        }
        let code = denial_code(&body);
        return Err(match (status.as_u16(), code.as_str()) {
            (401, "session_required" | "session_expired") => Denied::SessionEnded,
            (401, "invalid_grant") => Denied::GrantEnded,
            _ => Denied::Refused(code),
        });
    }
    Err(Denied::Unavailable)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// The adapter base URL: HTTPS, or HTTP on loopback; no query, fragment or
/// credentials. Returned without a trailing slash.
fn adapter_base(raw: &str) -> Result<String, CliError> {
    let bad =
        || CliError::Usage("--adapter must be an https:// URL with no query or fragment".into());
    let url = url::Url::parse(raw.trim()).map_err(|_| bad())?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"));
    if !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(bad());
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

/// NIP-FA's canonical `relay_url` for the CLI's relay: `wss://host[:port]`.
fn canonical_relay(relay_url: &str) -> Result<String, CliError> {
    let bad = || {
        CliError::Usage(format!(
            "NIP-FI needs a secure relay root (https:// or wss://), not {relay_url}"
        ))
    };
    let url = url::Url::parse(&relay_url.replacen("wss://", "https://", 1)).map_err(|_| bad())?;
    if url.scheme() != "https"
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port() == Some(80)
        || !url.username().is_empty()
    {
        return Err(bad());
    }
    let host = url.host_str().ok_or_else(bad)?;
    Ok(match url.port() {
        Some(port) => format!("wss://{host}:{port}"),
        None => format!("wss://{host}"),
    })
}

/// Local misuse checks on a returned assertion. The relay verifies it.
struct Expect<'a> {
    typ: &'a str,
    pubkey: &'a str,
    owner: Option<&'a str>,
    issuer: Option<&'a str>,
    relay: &'a str,
    expires_at: i64,
    not_after: Option<i64>,
}

fn check_assertion(jws: &str, expect: &Expect<'_>) -> Result<(), CliError> {
    let invalid = || CliError::Auth("the identity service returned an invalid assertion".into());
    let parts: Vec<&str> = jws.split('.').collect();
    if jws.len() > 8192 || parts.len() != 3 || parts.iter().any(|p| p.is_empty()) {
        return Err(invalid());
    }
    let decode = |part: &str| -> Result<serde_json::Value, CliError> {
        let bytes = URL_SAFE_NO_PAD.decode(part).map_err(|_| invalid())?;
        serde_json::from_slice(&bytes).map_err(|_| invalid())
    };
    let (header, claims) = (decode(parts[0])?, decode(parts[1])?);
    let text = |name: &str| claims.get(name).and_then(serde_json::Value::as_str);
    let num = |name: &str| claims.get(name).and_then(serde_json::Value::as_i64);
    let now = now();
    let audience = format!("https://{}", &expect.relay["wss://".len()..]);
    let ok = header.get("typ").and_then(serde_json::Value::as_str) == Some(expect.typ)
        && text("nostr_pubkey") == Some(expect.pubkey)
        && text("aud") == Some(&audience)
        && expect
            .owner
            .is_none_or(|o| text("nostr_owner_pubkey") == Some(o))
        && expect.issuer.is_none_or(|i| text("iss") == Some(i))
        && num("exp") == Some(expect.expires_at)
        && expect.expires_at > now
        && expect
            .not_after
            .is_none_or(|limit| expect.expires_at <= limit)
        && num("iat").is_some_and(|iat| iat <= now + 5 && iat <= expect.expires_at);
    if ok {
        Ok(())
    } else {
        Err(invalid())
    }
}

#[derive(Deserialize)]
struct Issued {
    assertion: String,
    nostr_pubkey: String,
    expires_at: i64,
}

fn json_body(pubkey: &str, relay: &str) -> Vec<u8> {
    // Field order is the NIP-FA request contract.
    format!(r#"{{"nostr_pubkey":"{pubkey}","relay_url":"{relay}"}}"#).into_bytes()
}

// ── Minting ──────────────────────────────────────────────────────────────────

/// Mint one assertion for `keys` at `relay_url`. With `owner` (the NIP-OA
/// owner from `BUZZ_AUTH_TAG`) the key is an agent and only its grant is used;
/// an agent never falls back to its owner's session.
pub async fn mint(
    store: &Store,
    keys: &Keys,
    owner: Option<&str>,
    relay_url: &str,
) -> Result<String, CliError> {
    let relay = canonical_relay(relay_url)?;
    let pubkey = keys.public_key().to_hex();
    let state = store.read()?;
    let client = adapter_client()?;
    match owner {
        Some(owner) => {
            let grant = state.grants.get(&pubkey).cloned().ok_or_else(|| {
                CliError::Auth(
                    "this agent is not enrolled for federated sign-in; run `buzz identity enroll-agent`"
                        .into(),
                )
            })?;
            if grant.owner != owner {
                return Err(CliError::Auth(
                    "this agent's grant belongs to a different owner; run `buzz identity enroll-agent`"
                        .into(),
                ));
            }
            if grant.expires_at.is_some_and(|at| at <= now()) {
                return Err(Denied::GrantEnded.into_error("agent sign-in"));
            }
            let url = format!("{}/v1/nip-fi/agent-assertion", grant.adapter);
            let body = json_body(&pubkey, &relay);
            let issued: Issued = match adapter_json(RETRY_ATTEMPTS, || {
                Ok(client
                    .post(&url)
                    .bearer_auth(&grant.token)
                    .header(
                        "Nostr-Authorization",
                        sign_nip98(keys, "POST", &url, Some(&body))?,
                    )
                    .header("Content-Type", "application/json")
                    .body(body.clone()))
            })
            .await
            {
                Ok(issued) => issued,
                Err(Denied::GrantEnded) => {
                    store.update(|state| {
                        let current = state.grants.get(&pubkey) == Some(&grant);
                        if current {
                            state.grants.remove(&pubkey);
                        }
                        Ok(((), current))
                    })?;
                    return Err(Denied::GrantEnded.into_error("agent sign-in"));
                }
                Err(denied) => return Err(denied.into_error("agent sign-in")),
            };
            if issued.nostr_pubkey != pubkey {
                return Err(Denied::Refused("invalid_response".into()).into_error("agent sign-in"));
            }
            check_assertion(
                &issued.assertion,
                &Expect {
                    typ: "nip-fi-agent+jwt",
                    pubkey: &pubkey,
                    owner: Some(&grant.owner),
                    issuer: Some(&grant.issuer),
                    relay: &relay,
                    expires_at: issued.expires_at,
                    not_after: None,
                },
            )?;
            Ok(issued.assertion)
        }
        None => {
            let session = state.session.ok_or_else(|| {
                CliError::Auth(
                    "not signed in for federated identity; run `buzz identity login`".into(),
                )
            })?;
            if session.expires_at <= now() {
                return Err(Denied::SessionEnded.into_error("sign-in"));
            }
            let url = format!("{}/v1/identity/assertions", session.adapter);
            let body = json_body(&pubkey, &relay);
            let issued: Issued = match adapter_json(RETRY_ATTEMPTS, || {
                Ok(client
                    .post(&url)
                    .bearer_auth(&session.token)
                    .header(
                        "Nostr-Authorization",
                        sign_nip98(keys, "POST", &url, Some(&body))?,
                    )
                    .header("Content-Type", "application/json")
                    .body(body.clone()))
            })
            .await
            {
                Ok(issued) => issued,
                Err(Denied::SessionEnded) => {
                    store.clear_session(&session)?;
                    return Err(Denied::SessionEnded.into_error("sign-in"));
                }
                Err(denied) => return Err(denied.into_error("sign-in")),
            };
            if issued.nostr_pubkey != pubkey {
                return Err(Denied::Refused("invalid_response".into()).into_error("sign-in"));
            }
            check_assertion(
                &issued.assertion,
                &Expect {
                    typ: "nip-fi+jwt",
                    pubkey: &pubkey,
                    owner: None,
                    issuer: None,
                    relay: &relay,
                    expires_at: issued.expires_at,
                    not_after: Some(session.expires_at),
                },
            )?;
            Ok(issued.assertion)
        }
    }
}

// ── Login ────────────────────────────────────────────────────────────────────

const LOGIN_WAIT: Duration = Duration::from_secs(300);

#[derive(Deserialize)]
struct Exchanged {
    session_token: String,
    expires_at: String,
}

#[derive(Deserialize)]
struct Checked {
    expires_at: String,
}

fn instant(rfc3339: &str) -> Result<i64, CliError> {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|t| t.timestamp())
        .map_err(|_| CliError::Auth("the identity service returned an invalid expiry".into()))
}

/// Wait on `listener` for the one browser redirect to `path` and return its
/// query parameters. Anything else gets a 404 and is ignored.
async fn await_callback(
    listener: tokio::net::TcpListener,
    path: &str,
) -> Result<Vec<(String, String)>, CliError> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|e| CliError::Other(e.to_string()))?;
        let mut buf = vec![0u8; 8192];
        let mut len = 0;
        while len < buf.len() && !buf[..len].windows(4).any(|w| w == b"\r\n\r\n") {
            match tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf[len..])).await {
                Ok(Ok(n)) if n > 0 => len += n,
                _ => break,
            }
        }
        let head = String::from_utf8_lossy(&buf[..len]);
        let target = head
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("GET "))
            .and_then(|rest| rest.split(' ').next())
            .unwrap_or_default();
        let parsed = url::Url::parse(&format!("http://127.0.0.1{target}")).ok();
        if parsed.as_ref().is_none_or(|u| u.path() != path) {
            let _ = stream
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
            continue;
        }
        let page = "<!doctype html><title>Buzz</title><p>Signed in. You can close this tab and return to the terminal.</p>";
        let _ = stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
                    page.len()
                )
                .as_bytes(),
            )
            .await;
        return Ok(parsed
            .map(|u| u.query_pairs().into_owned().collect())
            .unwrap_or_default());
    }
}

/// Run NIP-FA browser login against `adapter` and save the session.
/// `open` shows the login URL to the user (normally: open a browser).
pub async fn login(
    store: &Store,
    adapter: &str,
    protocol: Protocol,
    open: impl FnOnce(&str),
) -> Result<serde_json::Value, CliError> {
    let adapter = adapter_base(adapter)?;
    let secret = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(secret.as_bytes()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| CliError::Other(format!("could not open the login callback: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| CliError::Other(e.to_string()))?
        .port();
    let path = format!("/callback/{}", uuid::Uuid::new_v4());
    let mut start = url::Url::parse(&format!("{adapter}/v1/login/start"))
        .map_err(|e| CliError::Other(e.to_string()))?;
    start
        .query_pairs_mut()
        .append_pair("return_to", &format!("http://127.0.0.1:{port}{path}"))
        .append_pair("handoff_challenge", &challenge)
        .append_pair("handoff_challenge_method", "S256");
    open(start.as_str());

    let params = tokio::time::timeout(LOGIN_WAIT, await_callback(listener, &path))
        .await
        .map_err(|_| CliError::Auth("sign-in timed out".into()))??;
    if params.iter().any(|(k, _)| k == "error") {
        return Err(CliError::Auth("sign-in was refused or canceled".into()));
    }
    let mut codes = params
        .into_iter()
        .filter(|(k, _)| k == "code")
        .map(|(_, v)| v);
    let code = match (codes.next(), codes.next()) {
        (Some(code), None) if !code.is_empty() && code.len() <= 4096 => code,
        _ => return Err(CliError::Auth("sign-in returned no usable code".into())),
    };

    let client = adapter_client()?;
    let body = serde_json::json!({ "code": code, "handoff_secret": secret }).to_string();
    let exchange_url = format!("{adapter}/v1/login/exchange");
    // The code is single-use: one attempt only.
    let exchanged: Exchanged = adapter_json(1, || {
        Ok(client
            .post(&exchange_url)
            .header("Content-Type", "application/json")
            .body(body.clone()))
    })
    .await
    .map_err(|d| d.into_error("sign-in"))?;
    let token = exchanged.session_token;
    if token.is_empty() || token.len() > 4096 || !token.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(Denied::Refused("invalid_response".into()).into_error("sign-in"));
    }
    let expires_at = instant(&exchanged.expires_at)?;

    let session_url = format!("{adapter}/v1/session");
    let checked: Checked = adapter_json(RETRY_ATTEMPTS, || {
        Ok(client.get(&session_url).bearer_auth(&token))
    })
    .await
    .map_err(|d| d.into_error("sign-in"))?;
    // NIP-FA: the exchange and the session check must describe one session.
    if instant(&checked.expires_at)? != expires_at || expires_at <= now() {
        return Err(CliError::Auth(
            "the identity service described two different sessions; sign-in not saved".into(),
        ));
    }

    let session = Session {
        adapter: adapter.clone(),
        protocol,
        token,
        expires_at,
    };
    store.update(|state| {
        state.session = Some(session);
        Ok(((), true))
    })?;
    Ok(
        serde_json::json!({ "signed_in": true, "adapter": adapter, "expires_at": exchanged.expires_at }),
    )
}

// ── Agent enrollment ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Enrolled {
    access_token: String,
    issued_token_type: String,
    token_type: String,
    nostr_owner_pubkey: String,
    nip_fi_issuer: String,
    #[serde(default)]
    expires_in: Option<i64>,
}

/// Exchange the owner's session for a grant for the agent `keys`, proven by
/// `auth_tag_json` (the agent's verified NIP-OA tag) and the agent's signature.
pub async fn enroll_agent(
    store: &Store,
    keys: &Keys,
    owner: &str,
    auth_tag_json: &str,
) -> Result<serde_json::Value, CliError> {
    let session = store.read()?.session.ok_or_else(|| {
        CliError::Auth("the owner must sign in first; run `buzz identity login`".into())
    })?;
    if session.expires_at <= now() {
        return Err(Denied::SessionEnded.into_error("enrollment"));
    }
    let agent = keys.public_key().to_hex();
    let protocol = session.protocol;
    let mut form = url::form_urlencoded::Serializer::new(String::new());
    form.append_pair(
        "grant_type",
        "urn:ietf:params:oauth:grant-type:token-exchange",
    )
    .append_pair("subject_token", &session.token)
    .append_pair("subject_token_type", protocol.subject_token_type())
    .append_pair("requested_token_type", protocol.grant_token_type())
    .append_pair("agent_pubkey", &agent);
    if protocol == Protocol::Builderlab {
        form.append_pair("runtime", "local");
    }
    let form = form
        .append_pair("owner_attestation", auth_tag_json)
        .finish()
        .into_bytes();
    if form.len() > 8192 {
        return Err(CliError::Usage("enrollment request is too large".into()));
    }
    let url = format!("{}/v1/nip-fi/agent-grant", session.adapter);
    let client = adapter_client()?;
    let enrolled: Enrolled = match adapter_json(RETRY_ATTEMPTS, || {
        Ok(client
            .post(&url)
            .header(
                protocol.grant_proof_header(),
                sign_nip98(keys, "POST", &url, Some(&form))?,
            )
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(form.clone()))
    })
    .await
    {
        Ok(enrolled) => enrolled,
        Err(Denied::SessionEnded) => {
            store.clear_session(&session)?;
            return Err(Denied::SessionEnded.into_error("enrollment"));
        }
        Err(denied) => return Err(denied.into_error("enrollment")),
    };
    let token = &enrolled.access_token;
    if enrolled.issued_token_type != protocol.grant_token_type()
        || enrolled.token_type != "N_A"
        || enrolled.nostr_owner_pubkey != owner
        || enrolled.nip_fi_issuer.is_empty()
        || token.is_empty()
        || token.len() > 4096
        || !token.bytes().all(|b| b.is_ascii_graphic())
    {
        return Err(Denied::Refused("invalid_response".into()).into_error("enrollment"));
    }
    let grant = Grant {
        adapter: session.adapter.clone(),
        protocol,
        agent: agent.clone(),
        owner: owner.to_owned(),
        issuer: enrolled.nip_fi_issuer.clone(),
        token: enrolled.access_token,
        expires_at: enrolled.expires_in.map(|s| now().saturating_add(s)),
    };
    // Install only if the session that enrolled is still the one signed in.
    store.update(|state| {
        if state.session.as_ref() != Some(&session) {
            return Err(CliError::Auth(
                "the sign-in changed during enrollment; try again".into(),
            ));
        }
        state.grants.insert(agent.clone(), grant);
        Ok(((), true))
    })?;
    Ok(
        serde_json::json!({ "enrolled": true, "agent": agent, "owner": owner, "issuer": enrolled.nip_fi_issuer }),
    )
}

/// What is stored on this device, without any secret.
pub fn status(store: &Store) -> Result<serde_json::Value, CliError> {
    let state = store.read()?;
    let session = state.session.map(|s| {
        serde_json::json!({ "adapter": s.adapter, "protocol": s.protocol, "expires_at": s.expires_at, "expired": s.expires_at <= now() })
    });
    let grants: Vec<_> = state
        .grants
        .values()
        .map(|g| serde_json::json!({ "agent": g.agent, "owner": g.owner, "adapter": g.adapter, "issuer": g.issuer, "expires_at": g.expires_at }))
        .collect();
    Ok(serde_json::json!({ "session": session, "agents": grants }))
}

/// Journeys against a fake NIP-FA adapter on loopback: real HTTP through the
/// production functions, real files in a temporary store.
#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::body::Bytes;
    use axum::extract::{OriginalUri, State};
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, post};
    use axum::Router;

    use super::*;

    const RELAY: &str = "https://relay.example";
    const ISSUER: &str = "https://issuer.example";

    /// What the fake adapter answers and what it saw.
    #[derive(Default)]
    struct Fake {
        /// Override for one route: (status, body).
        answers: BTreeMap<&'static str, (u16, String)>,
        /// Session-check expiry, when it should differ from the exchange's.
        session_expiry: Option<String>,
        seen: Vec<(String, HeaderMap, String)>,
        /// Owner the adapter binds; the grant response echoes it.
        owner: String,
    }
    type Shared = Arc<Mutex<Fake>>;

    fn jws(header: serde_json::Value, claims: serde_json::Value) -> String {
        let enc = |v: serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
        format!("{}.{}.c2ln", enc(header), enc(claims))
    }

    const EXPIRY: &str = "2099-01-01T00:00:00Z";

    fn issued(typ: &str, pubkey: &str, owner: Option<&str>) -> String {
        let exp = now() + 300;
        let mut claims = serde_json::json!({
            "iss": ISSUER, "sub": "s", "aud": RELAY, "nostr_pubkey": pubkey,
            "iat": now(), "exp": exp,
        });
        if let Some(owner) = owner {
            claims["nostr_owner_pubkey"] = owner.into();
        }
        serde_json::json!({
            "assertion": jws(serde_json::json!({"alg":"ES256","typ":typ}), claims),
            "nostr_pubkey": pubkey, "expires_at": exp,
        })
        .to_string()
    }

    async fn handle(
        State(fake): State<Shared>,
        OriginalUri(uri): OriginalUri,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        let path = uri.path().to_owned();
        let body = String::from_utf8_lossy(&body).into_owned();
        let mut f = fake.lock().unwrap();
        f.seen.push((path.clone(), headers.clone(), body.clone()));
        if let Some((status, answer)) = f.answers.get(path.as_str()) {
            return (StatusCode::from_u16(*status).unwrap(), answer.clone()).into_response();
        }
        let ok = |b: String| (StatusCode::OK, b).into_response();
        let pubkey = || {
            serde_json::from_str::<serde_json::Value>(&body).unwrap()["nostr_pubkey"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        match path.as_str() {
            "/v1/login/exchange" => ok(serde_json::json!({"session_token":"sess-1","expires_at":EXPIRY}).to_string()),
            "/v1/session" => ok(serde_json::json!({"expires_at": f.session_expiry.clone().unwrap_or(EXPIRY.into())}).to_string()),
            "/v1/identity/assertions" => ok(issued("nip-fi+jwt", &pubkey(), None)),
            "/v1/nip-fi/agent-grant" => {
                let form: BTreeMap<String, String> =
                    url::form_urlencoded::parse(body.as_bytes()).into_owned().collect();
                ok(serde_json::json!({
                    "access_token": "grant-1",
                    "issued_token_type": form["requested_token_type"],
                    "token_type": "N_A",
                    "expires_in": 3600,
                    "nostr_owner_pubkey": f.owner,
                    "nip_fi_issuer": ISSUER,
                }).to_string())
            }
            "/v1/nip-fi/agent-assertion" => {
                let owner = f.owner.clone();
                ok(issued("nip-fi-agent+jwt", &pubkey(), Some(&owner)))
            }
            _ => StatusCode::NOT_FOUND.into_response(),
        }
    }

    async fn adapter(fake: Shared) -> String {
        let app = Router::new()
            .route("/{*path}", post(handle).get(handle))
            .route("/v1/session", get(handle))
            .with_state(fake);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    /// Complete the browser step: follow the login URL's return_to with a code.
    fn browser(code: &'static str) -> impl FnOnce(&str) {
        move |login: &str| {
            let login = url::Url::parse(login).unwrap();
            let q: BTreeMap<String, String> = login.query_pairs().into_owned().collect();
            assert_eq!(q["handoff_challenge_method"], "S256");
            let callback = format!("{}?code={code}", q["return_to"]);
            assert!(callback.starts_with("http://127.0.0.1:"));
            tokio::spawn(async move {
                reqwest::get(callback).await.unwrap();
            });
        }
    }

    fn setup() -> (tempfile::TempDir, Store, Shared, Keys, Keys) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path().join("buzz-cli"));
        let owner = Keys::generate();
        let fake = Shared::default();
        fake.lock().unwrap().owner = owner.public_key().to_hex();
        (dir, store, fake, owner, Keys::generate())
    }

    fn seen(fake: &Shared, path: &str) -> Vec<(HeaderMap, String)> {
        fake.lock()
            .unwrap()
            .seen
            .iter()
            .filter(|(p, ..)| p == path)
            .map(|(_, h, b)| (h.clone(), b.clone()))
            .collect()
    }

    #[tokio::test]
    async fn human_login_then_mint_for_own_key() {
        let (_dir, store, fake, owner, _) = setup();
        let base = adapter(fake.clone()).await;
        login(&store, &base, Protocol::NipFa, browser("code-1"))
            .await
            .unwrap();

        // The exchange carried the code and the secret whose S256 was the challenge.
        let (_, body) = seen(&fake, "/v1/login/exchange").remove(0);
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["code"], "code-1");
        let (headers, _) = seen(&fake, "/v1/session").remove(0);
        assert_eq!(headers["authorization"], "Bearer sess-1");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode =
                |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&store.dir), 0o700);
            assert_eq!(mode(&store.file()), 0o600);
        }

        let jws = mint(&store, &owner, None, "wss://relay.example/")
            .await
            .unwrap();
        let (headers, body) = seen(&fake, "/v1/identity/assertions").remove(0);
        assert_eq!(headers["authorization"], "Bearer sess-1");
        assert!(headers["nostr-authorization"]
            .to_str()
            .unwrap()
            .starts_with("Nostr "));
        assert_eq!(
            body,
            format!(
                r#"{{"nostr_pubkey":"{}","relay_url":"wss://relay.example"}}"#,
                owner.public_key().to_hex()
            )
        );
        assert_eq!(jws.split('.').count(), 3);
    }

    #[tokio::test]
    async fn login_refuses_two_different_session_expiries() {
        let (_dir, store, fake, ..) = setup();
        fake.lock().unwrap().session_expiry = Some("2099-01-01T00:00:01Z".into());
        let base = adapter(fake.clone()).await;
        let err = login(&store, &base, Protocol::NipFa, browser("c"))
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::Auth(_)), "{err:?}");
        assert!(!store.has_credentials());

        // The same instant in another notation is the same session.
        fake.lock().unwrap().session_expiry = Some("2098-12-31T19:00:00-05:00".into());
        login(&store, &base, Protocol::NipFa, browser("c"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn agent_uses_only_its_grant_never_its_owners_session() {
        for protocol in [Protocol::NipFa, Protocol::Builderlab] {
            let (_dir, store, fake, owner, agent) = setup();
            let base = adapter(fake.clone()).await;
            let owner_hex = owner.public_key().to_hex();
            login(&store, &base, protocol, browser("c")).await.unwrap();

            // Not enrolled: refused, and the human route is never asked.
            let err = mint(&store, &agent, Some(&owner_hex), RELAY)
                .await
                .unwrap_err();
            assert!(err.to_string().contains("enroll-agent"), "{err}");
            assert!(seen(&fake, "/v1/identity/assertions").is_empty());

            let tag = r#"["auth","o","","s"]"#;
            enroll_agent(&store, &agent, &owner_hex, tag).await.unwrap();
            let (headers, form) = seen(&fake, "/v1/nip-fi/agent-grant").remove(0);
            let fields: Vec<(String, String)> = url::form_urlencoded::parse(form.as_bytes())
                .into_owned()
                .collect();
            let names: Vec<&str> = fields.iter().map(|(k, _)| k.as_str()).collect();
            assert!(headers.get("authorization").is_none());
            match protocol {
                Protocol::NipFa => {
                    assert_eq!(
                        names,
                        [
                            "grant_type",
                            "subject_token",
                            "subject_token_type",
                            "requested_token_type",
                            "agent_pubkey",
                            "owner_attestation"
                        ]
                    );
                    assert_eq!(
                        fields[2].1,
                        "urn:nostr:params:oauth:token-type:nip-fa-session"
                    );
                    assert!(headers.contains_key("nostr-authorization"));
                    assert!(!headers.contains_key("nostr-agent-proof"));
                }
                Protocol::Builderlab => {
                    assert_eq!(
                        names,
                        [
                            "grant_type",
                            "subject_token",
                            "subject_token_type",
                            "requested_token_type",
                            "agent_pubkey",
                            "runtime",
                            "owner_attestation"
                        ]
                    );
                    assert_eq!(
                        fields[2].1,
                        "urn:builderlab:params:oauth:token-type:bb-session"
                    );
                    assert!(headers.contains_key("nostr-agent-proof"));
                    assert!(!headers.contains_key("nostr-authorization"));
                }
            }
            assert_eq!(fields[1].1, "sess-1");
            assert_eq!(
                fields[5 + (protocol == Protocol::Builderlab) as usize].1,
                tag
            );

            mint(&store, &agent, Some(&owner_hex), RELAY).await.unwrap();
            let (headers, _) = seen(&fake, "/v1/nip-fi/agent-assertion").remove(0);
            assert_eq!(headers["authorization"], "Bearer grant-1");
            assert!(seen(&fake, "/v1/identity/assertions").is_empty());

            // A grant for a different owner is not used.
            let other = Keys::generate().public_key().to_hex();
            assert!(mint(&store, &agent, Some(&other), RELAY).await.is_err());
        }
    }

    #[tokio::test]
    async fn grant_response_and_agent_assertion_are_checked_against_the_binding() {
        let (_dir, store, fake, owner, agent) = setup();
        let base = adapter(fake.clone()).await;
        let owner_hex = owner.public_key().to_hex();
        login(&store, &base, Protocol::NipFa, browser("c"))
            .await
            .unwrap();

        // A grant naming another owner is refused and not stored.
        fake.lock().unwrap().owner = Keys::generate().public_key().to_hex();
        assert!(enroll_agent(&store, &agent, &owner_hex, "[]")
            .await
            .is_err());
        assert!(store.read().unwrap().grants.is_empty());

        fake.lock().unwrap().owner = owner_hex.clone();
        enroll_agent(&store, &agent, &owner_hex, "[]")
            .await
            .unwrap();
        // The adapter now issues assertions for a different owner: rejected.
        fake.lock().unwrap().owner = Keys::generate().public_key().to_hex();
        let err = mint(&store, &agent, Some(&owner_hex), RELAY)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid assertion"), "{err}");
        // The grant survives a malformed answer.
        assert_eq!(store.read().unwrap().grants.len(), 1);
    }

    #[tokio::test]
    async fn denials_clear_only_the_credential_that_ended() {
        let (_dir, store, fake, owner, agent) = setup();
        let base = adapter(fake.clone()).await;
        let owner_hex = owner.public_key().to_hex();
        login(&store, &base, Protocol::NipFa, browser("c"))
            .await
            .unwrap();
        enroll_agent(&store, &agent, &owner_hex, "[]")
            .await
            .unwrap();
        let answer = |path: &'static str, status: u16, code: &str| {
            fake.lock()
                .unwrap()
                .answers
                .insert(path, (status, format!(r#"{{"error":"{code}"}}"#)));
        };

        // Refusals and unknown codes keep everything.
        for (status, code) in [
            (403, "authorization_denied"),
            (401, "whatever"),
            (400, "invalid_request"),
        ] {
            answer("/v1/nip-fi/agent-assertion", status, code);
            assert!(mint(&store, &agent, Some(&owner_hex), RELAY).await.is_err());
            answer("/v1/identity/assertions", status, code);
            assert!(mint(&store, &owner, None, RELAY).await.is_err());
            let state = store.read().unwrap();
            assert!(
                state.session.is_some() && state.grants.len() == 1,
                "{status} {code}"
            );
        }

        // invalid_grant ends the grant only.
        answer("/v1/nip-fi/agent-assertion", 401, "invalid_grant");
        mint(&store, &agent, Some(&owner_hex), RELAY)
            .await
            .unwrap_err();
        let state = store.read().unwrap();
        assert!(state.grants.is_empty() && state.session.is_some());

        // session_expired ends the session only.
        enroll_agent(&store, &agent, &owner_hex, "[]")
            .await
            .unwrap();
        answer("/v1/identity/assertions", 401, "session_expired");
        mint(&store, &owner, None, RELAY).await.unwrap_err();
        let state = store.read().unwrap();
        assert!(state.session.is_none() && state.grants.len() == 1);
    }

    #[tokio::test]
    async fn unavailable_adapter_retries_with_fresh_proofs_then_fails_closed() {
        let (_dir, store, fake, owner, _) = setup();
        let base = adapter(fake.clone()).await;
        login(&store, &base, Protocol::NipFa, browser("c"))
            .await
            .unwrap();
        fake.lock()
            .unwrap()
            .answers
            .insert("/v1/identity/assertions", (503, "{}".into()));
        let err = mint(&store, &owner, None, RELAY).await.unwrap_err();
        assert!(err.to_string().contains("unavailable"), "{err}");
        let proofs: Vec<String> = seen(&fake, "/v1/identity/assertions")
            .into_iter()
            .map(|(h, _)| h["nostr-authorization"].to_str().unwrap().to_owned())
            .collect();
        assert_eq!(proofs.len(), RETRY_ATTEMPTS as usize);
        assert!(proofs[0] != proofs[1] && proofs[1] != proofs[2]);
        assert!(store.read().unwrap().session.is_some());
    }

    #[tokio::test]
    async fn redirects_are_never_followed_by_adapter_calls() {
        let (_dir, store, fake, owner, _) = setup();
        let elsewhere = Shared::default();
        let other = adapter(elsewhere.clone()).await;
        let base = adapter(fake.clone()).await;
        login(&store, &base, Protocol::NipFa, browser("c"))
            .await
            .unwrap();
        // A 307 would replay the session to another origin.
        let app = Router::new().route(
            "/v1/identity/assertions",
            post(move || async move {
                (
                    StatusCode::TEMPORARY_REDIRECT,
                    [("location", format!("{other}/v1/identity/assertions"))],
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let redirecting = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        store
            .update(|s| {
                s.session.as_mut().unwrap().adapter = redirecting;
                Ok(((), true))
            })
            .unwrap();
        assert!(mint(&store, &owner, None, RELAY).await.is_err());
        assert!(elsewhere.lock().unwrap().seen.is_empty());
    }

    #[tokio::test]
    async fn switching_adapters_never_sends_a_grant_to_the_new_one() {
        let (_dir, store, fake, owner, agent) = setup();
        let first = adapter(fake.clone()).await;
        let owner_hex = owner.public_key().to_hex();
        login(&store, &first, Protocol::NipFa, browser("c"))
            .await
            .unwrap();
        enroll_agent(&store, &agent, &owner_hex, "[]")
            .await
            .unwrap();

        let second_fake = Shared::default();
        second_fake.lock().unwrap().owner = owner_hex.clone();
        let second = adapter(second_fake.clone()).await;
        login(&store, &second, Protocol::NipFa, browser("c"))
            .await
            .unwrap();

        mint(&store, &agent, Some(&owner_hex), RELAY).await.unwrap();
        assert!(second_fake
            .lock()
            .unwrap()
            .seen
            .iter()
            .all(|(p, ..)| !p.contains("agent")));
        assert_eq!(seen(&fake, "/v1/nip-fi/agent-assertion").len(), 1);
    }

    #[test]
    fn relay_url_is_canonical_and_secure() {
        for (input, want) in [
            ("https://Relay.Example/", "wss://relay.example"),
            ("wss://relay.example:8443", "wss://relay.example:8443"),
            ("https://relay.example:443", "wss://relay.example"),
        ] {
            assert_eq!(canonical_relay(input).unwrap(), want);
        }
        for bad in [
            "http://relay.example",
            "https://relay.example/path",
            "https://relay.example:80",
            "https://relay.example/?q",
        ] {
            assert!(canonical_relay(bad).is_err(), "{bad}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn store_refuses_symlinks_and_logout_removes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path().join("buzz-cli"));
        store
            .update(|s| {
                s.session = Some(Session {
                    adapter: "a".into(),
                    protocol: Protocol::NipFa,
                    token: "t".into(),
                    expires_at: 1,
                });
                Ok(((), true))
            })
            .unwrap();
        store.logout().unwrap();
        assert!(!store.has_credentials());

        let target = dir.path().join("elsewhere");
        std::fs::write(&target, "{}").unwrap();
        std::os::unix::fs::symlink(&target, store.file()).unwrap();
        assert!(store.read().is_err());
    }
}
