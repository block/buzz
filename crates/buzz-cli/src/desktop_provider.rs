//! The desktop link of the credential chain: a running Buzz Desktop acts as
//! one identity over loopback (`docs/local-credential-provider.md`).
//!
//! The caller never holds the private key. It finds a provider through an
//! inherited `BUZZ_CREDENTIAL_PROVIDER` token or a paired token in the OS
//! keychain, and sends each request on a connection whose server side it has
//! confirmed belongs to the same OS user (inherited tokens skip that check,
//! because the desktop handed the endpoint to the process directly).
//!
//! Version 1 has no Windows transport, so the link is compiled for Unix only.

use std::sync::Arc;
use std::time::Duration;

use nostr::signer::{SignerBackend, SignerError};
use nostr::{Event, JsonUtil, PublicKey, UnsignedEvent};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

use crate::error::CliError;
use crate::federated_identity::canonical_relay_url;

pub const PROVIDER_ENV: &str = "BUZZ_CREDENTIAL_PROVIDER";

/// The protocol version this caller speaks.
const VERSION: u32 = 1;
const VERSION_HEADER: &str = "Buzz-Credential-Provider";

/// Methods the provider may hold open for a person's decision (up to 120s).
const HELD_TIMEOUT: Duration = Duration::from_secs(150);
const QUICK_TIMEOUT: Duration = Duration::from_secs(10);
const SAME_USER_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_RESPONSE_BYTES: usize = 256 * 1024;

/// Re-ask for an assertion this long before its `exp`.
const REFRESH_MARGIN_SECS: u64 = 30;

/// A provider failure, by the contract's error code.
#[derive(Debug)]
pub enum ProviderError {
    /// `revoked`: drop the token and fall through to the next source.
    Revoked,
    /// No common protocol version: the provider does not apply.
    NoCommonVersion,
    /// `unknown_method`: degrade where the caller can.
    UnknownMethod,
    /// Any other code. Unrecognized codes are reported as `denied`.
    Code {
        code: &'static str,
        message: Option<String>,
        retry_after: Option<u64>,
    },
    /// The provider could not be reached or answered outside the contract.
    Transport(String),
}

impl ProviderError {
    fn into_cli(self, method: &str) -> CliError {
        match self {
            Self::Revoked => CliError::Auth(
                "the Buzz desktop revoked this CLI's token; run `buzz auth desktop pair` again"
                    .into(),
            ),
            Self::NoCommonVersion => CliError::Auth(
                "the Buzz desktop speaks no credential-provider version this CLI supports".into(),
            ),
            Self::UnknownMethod => CliError::Auth(format!(
                "the Buzz desktop does not implement `{method}`; update Buzz"
            )),
            Self::Code {
                code,
                message,
                retry_after,
            } => {
                let detail = message.map(|m| format!(": {m}")).unwrap_or_default();
                match code {
                    "approval_required" => {
                        CliError::Auth(format!("approve this request in Buzz, then retry{detail}"))
                    }
                    "rate_limited" => CliError::Auth(format!(
                        "the Buzz desktop is rate-limiting this CLI; retry in {}s{detail}",
                        retry_after.unwrap_or(1)
                    )),
                    "unavailable" => {
                        CliError::Auth(format!("the Buzz desktop cannot act now{detail}"))
                    }
                    "invalid_request" => CliError::Other(format!(
                        "the Buzz desktop rejected `{method}` as malformed{detail}"
                    )),
                    _ => CliError::Auth(format!("the Buzz desktop refused `{method}`{detail}")),
                }
            }
            Self::Transport(why) => CliError::Auth(format!("Buzz desktop: {why}")),
        }
    }
}

/// The contract's codes; anything else is `denied`.
fn known_code(code: &str) -> &'static str {
    match code {
        "approval_required" => "approval_required",
        "rate_limited" => "rate_limited",
        "unavailable" => "unavailable",
        "invalid_request" => "invalid_request",
        "unsupported_version" => "unsupported_version",
        _ => "denied",
    }
}

fn held(method: &str) -> bool {
    matches!(method, "sign_event" | "nip44_decrypt" | "pair")
}

/// A provider endpoint: `http://127.0.0.1:<port>/<path>`.
#[derive(Clone, Debug)]
pub struct Endpoint {
    port: u16,
    path: String,
}

impl Endpoint {
    pub fn parse(raw: &str) -> Result<Self, CliError> {
        let invalid =
            || CliError::Auth("credential provider URL must be http://127.0.0.1:<port>/…".into());
        let url = url::Url::parse(raw).map_err(|_| invalid())?;
        if url.scheme() != "http"
            || url.host() != Some(url::Host::Ipv4(std::net::Ipv4Addr::LOCALHOST))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid());
        }
        let port = url.port().ok_or_else(invalid)?;
        let mut path = url.path().to_owned();
        if let Some(query) = url.query() {
            path = format!("{path}?{query}");
        }
        Ok(Self { port, path })
    }
}

/// Exchange one request on `stream` (one request per connection) and return
/// the HTTP status and body.
async fn exchange(
    mut stream: TcpStream,
    endpoint: &Endpoint,
    token: Option<&str>,
    version: u32,
    body: &[u8],
) -> Result<(u16, Vec<u8>), ProviderError> {
    let io =
        |e: std::io::Error| ProviderError::Transport(format!("connection failed: {}", e.kind()));
    let mut head = format!(
        "POST {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\n{VERSION_HEADER}: {version}\r\nConnection: close\r\n",
        endpoint.path,
        endpoint.port,
        body.len()
    );
    if let Some(token) = token {
        head.push_str(&format!("Authorization: Bearer {token}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await.map_err(io)?;
    stream.write_all(body).await.map_err(io)?;
    let mut raw = Vec::new();
    (&mut stream)
        .take(MAX_RESPONSE_BYTES as u64 + 1)
        .read_to_end(&mut raw)
        .await
        .map_err(io)?;
    if raw.len() > MAX_RESPONSE_BYTES {
        return Err(ProviderError::Transport("response too large".into()));
    }
    parse_response(&raw).ok_or_else(|| ProviderError::Transport("malformed HTTP response".into()))
}

/// Parse an HTTP/1.1 response read to EOF: status plus body, decoding
/// `Content-Length` or chunked framing.
fn parse_response(raw: &[u8]) -> Option<(u16, Vec<u8>)> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..split]).ok()?;
    let rest = &raw[split + 4..];
    let mut lines = head.split("\r\n");
    let status: u16 = lines.next()?.split(' ').nth(1)?.parse().ok()?;
    let mut length = None;
    let mut chunked = false;
    for line in lines {
        let (name, value) = line.split_once(':')?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            length = Some(value.parse::<usize>().ok()?);
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            chunked = value.eq_ignore_ascii_case("chunked");
        }
    }
    if chunked {
        let mut body = Vec::new();
        let mut at = rest;
        loop {
            let end = at.windows(2).position(|w| w == b"\r\n")?;
            let size_hex = std::str::from_utf8(&at[..end])
                .ok()?
                .split(';')
                .next()?
                .trim();
            let size = usize::from_str_radix(size_hex, 16).ok()?;
            at = &at[end + 2..];
            if size == 0 {
                return Some((status, body));
            }
            body.extend_from_slice(at.get(..size)?);
            at = at.get(size + 2..)?;
        }
    }
    match length {
        Some(n) => Some((status, rest.get(..n)?.to_vec())),
        None => Some((status, rest.to_vec())),
    }
}

// ---------------------------------------------------------------------------
// Same-user check
// ---------------------------------------------------------------------------

/// Connect to the provider port. With `check`, confirm by numeric user ID that
/// a process of this user holds the server side of this very connection, and
/// refuse to use the connection otherwise.
async fn connect(port: u16, check: bool) -> Result<TcpStream, ProviderError> {
    let stream = tokio::time::timeout(QUICK_TIMEOUT, TcpStream::connect(("127.0.0.1", port)))
        .await
        .map_err(|_| ProviderError::Transport("connection timed out".into()))?
        .map_err(|e| ProviderError::Transport(format!("not reachable: {}", e.kind())))?;
    if check {
        let local = stream
            .local_addr()
            .map_err(|_| ProviderError::Transport("no local address".into()))?
            .port();
        let confirmed = tokio::time::timeout(SAME_USER_TIMEOUT, server_side_uid(port, local))
            .await
            .ok()
            .flatten()
            == Some(current_uid());
        if !confirmed {
            return Err(ProviderError::Transport(
                "could not confirm the provider runs as this user; nothing was sent".into(),
            ));
        }
    }
    Ok(stream)
}

fn current_uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

/// The owner UID of the socket `127.0.0.1:server -> 127.0.0.1:client`.
#[cfg(target_os = "linux")]
async fn server_side_uid(server: u16, client: u16) -> Option<u32> {
    let table = tokio::fs::read_to_string("/proc/net/tcp").await.ok()?;
    proc_net_tcp_uid(&table, server, client)
}

#[cfg(any(target_os = "linux", test))]
fn proc_net_tcp_uid(table: &str, server: u16, client: u16) -> Option<u32> {
    let local = format!("0100007F:{server:04X}");
    let remote = format!("0100007F:{client:04X}");
    table.lines().skip(1).find_map(|line| {
        let cols: Vec<&str> = line.split_whitespace().collect();
        (cols.get(1) == Some(&local.as_str()) && cols.get(2) == Some(&remote.as_str()))
            .then(|| cols.get(7)?.parse().ok())
            .flatten()
    })
}

/// macOS: `lsof` lists the socket's owning process with its UID.
#[cfg(not(target_os = "linux"))]
async fn server_side_uid(server: u16, client: u16) -> Option<u32> {
    let output = tokio::process::Command::new("lsof")
        .args(["-nP", "-F", "un", &format!("-iTCP@127.0.0.1:{server}")])
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    lsof_uid(&String::from_utf8_lossy(&output.stdout), server, client)
}

#[cfg(any(not(target_os = "linux"), test))]
fn lsof_uid(output: &str, server: u16, client: u16) -> Option<u32> {
    let want = format!("n127.0.0.1:{server}->127.0.0.1:{client}");
    let mut uid = None;
    for line in output.lines() {
        if let Some(value) = line.strip_prefix('u') {
            uid = value.parse().ok();
        } else if line.strip_prefix('p').is_some() {
            uid = None;
        } else if line == want {
            return uid;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Provider client
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct Envelope {
    #[serde(default)]
    id: String,
    #[serde(default)]
    result: String,
    error: Option<String>,
    message: Option<String>,
    retry_after: Option<u64>,
    versions: Option<Vec<u32>>,
}

/// Send one envelope and return its `result`, negotiating the version once.
async fn call(
    endpoint: &Endpoint,
    token: Option<&str>,
    same_user: bool,
    method: &str,
    params: &[String],
) -> Result<String, ProviderError> {
    let mut version = VERSION;
    for _ in 0..2 {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let body = serde_json::to_vec(&serde_json::json!({
            "id": id, "method": method, "params": params,
        }))
        .map_err(|_| ProviderError::Transport("request serialization failed".into()))?;
        let timeout = if held(method) {
            HELD_TIMEOUT
        } else {
            QUICK_TIMEOUT
        };
        let attempt = async {
            let stream = connect(endpoint.port, same_user).await?;
            exchange(stream, endpoint, token, version, &body).await
        };
        let (_, raw) = tokio::time::timeout(timeout, attempt)
            .await
            .map_err(|_| ProviderError::Transport(format!("`{method}` timed out")))??;
        let envelope: Envelope = serde_json::from_slice(&raw)
            .map_err(|_| ProviderError::Transport("unreadable response".into()))?;
        // Decide on `error`, never on the HTTP status.
        let Some(code) = envelope.error.filter(|c| !c.is_empty()) else {
            if envelope.id != id {
                return Err(ProviderError::Transport("response id mismatch".into()));
            }
            return Ok(envelope.result);
        };
        match code.as_str() {
            "revoked" => return Err(ProviderError::Revoked),
            "unknown_method" => return Err(ProviderError::UnknownMethod),
            "unsupported_version" => {
                let common = envelope
                    .versions
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|v| *v <= VERSION && *v != version)
                    .max();
                match common {
                    Some(v) => version = v,
                    None => return Err(ProviderError::NoCommonVersion),
                }
            }
            other => {
                return Err(ProviderError::Code {
                    code: known_code(other),
                    message: envelope.message,
                    retry_after: envelope.retry_after,
                })
            }
        }
    }
    Err(ProviderError::NoCommonVersion)
}

#[derive(serde::Deserialize)]
struct Describe {
    pubkey: String,
    #[serde(default)]
    relays: Vec<String>,
    #[serde(default)]
    methods: Vec<String>,
}

struct CachedAssertion {
    relay_url: String,
    assertion: Option<String>,
    refresh_at: u64,
}

/// A connected provider acting as one identity.
pub struct Provider {
    endpoint: Endpoint,
    token: String,
    same_user: bool,
    public_key: PublicKey,
    methods: Vec<String>,
    cached: Mutex<Option<CachedAssertion>>,
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provider")
            .field("port", &self.endpoint.port)
            .field("public_key", &self.public_key.to_hex())
            .finish_non_exhaustive()
    }
}

/// Why a provider was not used for this relay.
pub enum Skip {
    /// The token was revoked: the caller drops it.
    Revoked,
    /// The provider does not apply (relay not bound, no common version).
    NotApplicable,
}

impl Provider {
    /// Connect with `token` and confirm the provider acts for `relay_url`.
    pub async fn open(
        endpoint: Endpoint,
        token: String,
        same_user: bool,
        relay_url: &str,
    ) -> Result<Result<Arc<Self>, Skip>, CliError> {
        let raw = match call(&endpoint, Some(&token), same_user, "describe", &[]).await {
            Ok(raw) => raw,
            Err(ProviderError::Revoked) => return Ok(Err(Skip::Revoked)),
            Err(ProviderError::NoCommonVersion) => return Ok(Err(Skip::NotApplicable)),
            Err(e) => return Err(e.into_cli("describe")),
        };
        let describe: Describe = serde_json::from_str(&raw)
            .map_err(|_| CliError::Auth("Buzz desktop: unreadable `describe` answer".into()))?;
        let public_key = PublicKey::from_hex(&describe.pubkey)
            .map_err(|_| CliError::Auth("Buzz desktop: invalid identity public key".into()))?;
        let relay = canonical_relay_url(relay_url)?;
        let bound = describe
            .relays
            .iter()
            .any(|r| canonical_relay_url(r).is_ok_and(|r| r == relay));
        if !bound {
            return Ok(Err(Skip::NotApplicable));
        }
        Ok(Ok(Arc::new(Self {
            endpoint,
            token,
            same_user,
            public_key,
            methods: describe.methods,
            cached: Mutex::new(None),
        })))
    }

    pub fn public_key(&self) -> PublicKey {
        self.public_key
    }

    async fn call(&self, method: &str, params: &[String]) -> Result<String, CliError> {
        call(
            &self.endpoint,
            Some(&self.token),
            self.same_user,
            method,
            params,
        )
        .await
        .map_err(|e| e.into_cli(method))
    }

    /// The identity's NIP-OA `auth` tag JSON, if it has one.
    pub async fn auth_tag(&self) -> Result<Option<String>, CliError> {
        if !self.methods.iter().any(|m| m == "auth_tag") {
            return Ok(None);
        }
        match call(
            &self.endpoint,
            Some(&self.token),
            self.same_user,
            "auth_tag",
            &[],
        )
        .await
        {
            Ok(tag) => Ok(Some(tag).filter(|t| !t.is_empty())),
            Err(ProviderError::UnknownMethod) => Ok(None),
            Err(e) => Err(e.into_cli("auth_tag")),
        }
    }

    /// The NIP-FI assertion for `relay_url`, cached until shortly before its
    /// `exp`. `None` when the desktop has none for that relay.
    pub async fn federated_identity(&self, relay_url: &str) -> Result<Option<String>, CliError> {
        let mut cached = self.cached.lock().await;
        if let Some(entry) = cached.as_ref() {
            if entry.relay_url == relay_url && unix_now() < entry.refresh_at {
                return Ok(entry.assertion.clone());
            }
        }
        let params = [relay_url.to_owned()];
        let assertion = match call(
            &self.endpoint,
            Some(&self.token),
            self.same_user,
            "federated_identity",
            &params,
        )
        .await
        {
            Ok(a) if a.is_empty() => None,
            Ok(a) => Some(a),
            Err(ProviderError::UnknownMethod) => None,
            Err(e) => return Err(e.into_cli("federated_identity")),
        };
        let refresh_at = match assertion.as_deref() {
            Some(a) => {
                let exp = jwt_exp(a).ok_or_else(|| {
                    CliError::Auth("Buzz desktop returned an unreadable assertion".into())
                })?;
                exp.saturating_sub(REFRESH_MARGIN_SECS)
            }
            // Ask again on the next command; within one, there is none.
            None => u64::MAX,
        };
        *cached = Some(CachedAssertion {
            relay_url: relay_url.to_owned(),
            assertion: assertion.clone(),
            refresh_at,
        });
        Ok(assertion)
    }

    /// The NIP-AE `d` tag for `slug`, derived by the desktop.
    pub async fn engram_d_tag(&self, peer: &PublicKey, slug: &str) -> Result<String, CliError> {
        let tag = self
            .call("engram_d_tag", &[peer.to_hex(), slug.to_owned()])
            .await?;
        if tag.len() != 64 || !tag.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(CliError::Auth(
                "Buzz desktop returned an invalid engram d tag".into(),
            ));
        }
        Ok(tag)
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The `exp` claim of a compact JWS, without verifying it (the relay does).
fn jwt_exp(jws: &str) -> Option<u64> {
    use base64::Engine;
    let mut parts = jws.split('.');
    let (_, payload, _, None) = (parts.next()?, parts.next()?, parts.next()?, parts.next()) else {
        return None;
    };
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()?
        .get("exp")?
        .as_u64()
}

fn signer_error(e: CliError) -> SignerError {
    SignerError::from(e.to_string())
}

impl nostr::NostrSigner for Provider {
    fn backend(&self) -> SignerBackend<'_> {
        SignerBackend::Custom("buzz-desktop".into())
    }

    fn get_public_key(&self) -> nostr::util::BoxedFuture<'_, Result<PublicKey, SignerError>> {
        Box::pin(async move { Ok(self.public_key) })
    }

    fn sign_event(
        &self,
        unsigned: UnsignedEvent,
    ) -> nostr::util::BoxedFuture<'_, Result<Event, SignerError>> {
        Box::pin(async move {
            let signed = self
                .call("sign_event", &[unsigned.as_json()])
                .await
                .map_err(signer_error)?;
            let event = Event::from_json(&signed)
                .map_err(|_| SignerError::from("Buzz desktop returned an unreadable event"))?;
            if event.pubkey != self.public_key || event.verify().is_err() {
                return Err("Buzz desktop returned an invalid signature".into());
            }
            Ok(event)
        })
    }

    fn nip04_encrypt<'a>(
        &'a self,
        _: &'a PublicKey,
        _: &'a str,
    ) -> nostr::util::BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async { Err("the Buzz desktop provider does not offer NIP-04".into()) })
    }

    fn nip04_decrypt<'a>(
        &'a self,
        _: &'a PublicKey,
        _: &'a str,
    ) -> nostr::util::BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async { Err("the Buzz desktop provider does not offer NIP-04".into()) })
    }

    fn nip44_encrypt<'a>(
        &'a self,
        peer: &'a PublicKey,
        content: &'a str,
    ) -> nostr::util::BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async move {
            self.call("nip44_encrypt", &[peer.to_hex(), content.to_owned()])
                .await
                .map_err(signer_error)
        })
    }

    fn nip44_decrypt<'a>(
        &'a self,
        peer: &'a PublicKey,
        payload: &'a str,
    ) -> nostr::util::BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async move {
            self.call("nip44_decrypt", &[peer.to_hex(), payload.to_owned()])
                .await
                .map_err(signer_error)
        })
    }
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct Inherited {
    url: String,
    token: String,
}

/// The inherited token from `BUZZ_CREDENTIAL_PROVIDER`, if set.
pub fn inherited() -> Result<Option<(Endpoint, String)>, CliError> {
    let Some(raw) = std::env::var(PROVIDER_ENV)
        .ok()
        .filter(|v| !v.trim().is_empty())
    else {
        return Ok(None);
    };
    let inherited: Inherited = serde_json::from_str(&raw)
        .map_err(|_| CliError::Auth(format!("{PROVIDER_ENV} is not valid JSON")))?;
    let endpoint = Endpoint::parse(&inherited.url)?;
    if !usable_token(&inherited.token) {
        return Err(CliError::Auth(format!(
            "{PROVIDER_ENV} carries an unusable token"
        )));
    }
    Ok(Some((endpoint, inherited.token)))
}

fn usable_token(token: &str) -> bool {
    !token.is_empty() && token.bytes().all(|b| b.is_ascii_graphic())
}

#[derive(serde::Deserialize)]
struct Advertisement {
    instance: String,
    url: String,
}

fn advertisement_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .map(|home| std::path::Path::new(&home).join(".buzz-foundation/credential-providers"))
}

/// Every advertisement that passes the ownership and permission checks:
/// a `0700` directory and files owned by this user with no group or other
/// bits. Anything else is ignored, so another user cannot plant one.
fn advertisements_in(dir: &std::path::Path) -> Vec<(String, Endpoint)> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let uid = current_uid();
    let private =
        |m: &std::fs::Metadata, mode: u32| m.uid() == uid && m.permissions().mode() & 0o777 == mode;
    let Ok(meta) = std::fs::symlink_metadata(dir) else {
        return Vec::new();
    };
    if !meta.is_dir() || !private(&meta, 0o700) {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(String, Endpoint)> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            !name.starts_with('.') && name.ends_with(".json")
        })
        .filter_map(|e| {
            let meta = std::fs::symlink_metadata(e.path()).ok()?;
            if !meta.is_file() || meta.uid() != uid || meta.permissions().mode() & 0o077 != 0 {
                return None;
            }
            let ad: Advertisement = serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok()?;
            Some((ad.instance, Endpoint::parse(&ad.url).ok()?))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

fn advertisements() -> Vec<(String, Endpoint)> {
    advertisement_dir().map_or_else(Vec::new, |d| advertisements_in(&d))
}

// ---------------------------------------------------------------------------
// Paired tokens, in the OS keychain
// ---------------------------------------------------------------------------

const KEYCHAIN_SERVICE: &str = "buzz-cli";
const KEYCHAIN_ACCOUNT: &str = "desktop-pairings";

/// One pairing per instance.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Pairings(std::collections::BTreeMap<String, Pairing>);

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct Pairing {
    token: String,
    pubkey: String,
    relays: Vec<String>,
}

fn keychain() -> Result<keyring::Entry, keyring::Error> {
    keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
}

fn no_keychain(e: keyring::Error) -> CliError {
    CliError::Auth(format!(
        "no OS keychain is reachable ({e}); on Linux start a Secret Service such as \
         gnome-keyring, or set BUZZ_PRIVATE_KEY instead. Paired tokens are never written to files."
    ))
}

fn load_pairings() -> Result<Pairings, CliError> {
    match keychain().and_then(|entry| entry.get_password()) {
        Ok(json) => Ok(serde_json::from_str(&json).unwrap_or_default()),
        Err(keyring::Error::NoEntry) => Ok(Pairings::default()),
        Err(e) => Err(no_keychain(e)),
    }
}

fn save_pairings(pairings: &Pairings) -> Result<(), CliError> {
    let entry = keychain().map_err(no_keychain)?;
    if pairings.0.is_empty() {
        return match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(no_keychain(e)),
        };
    }
    let json = serde_json::to_string(pairings)
        .map_err(|e| CliError::Other(format!("pairing serialization failed: {e}")))?;
    entry.set_password(&json).map_err(no_keychain)
}

// ---------------------------------------------------------------------------
// The chain link, and pairing
// ---------------------------------------------------------------------------

/// The desktop link for `relay_url`: the inherited token first, then a paired
/// token whose instance is advertised. `None` when neither applies, so the
/// chain moves on.
pub async fn discover(relay_url: &str) -> Result<Option<Arc<Provider>>, CliError> {
    if let Some((endpoint, token)) = inherited()? {
        match Provider::open(endpoint, token, false, relay_url).await? {
            Ok(provider) => return Ok(Some(provider)),
            Err(Skip::Revoked | Skip::NotApplicable) => {}
        }
    }
    // A missing keychain only means there are no paired tokens to use here.
    let Ok(mut pairings) = load_pairings() else {
        return Ok(None);
    };
    let relay = canonical_relay_url(relay_url)?;
    let mut revoked = false;
    for (instance, endpoint) in advertisements() {
        let Some(pairing) = pairings.0.get(&instance).cloned() else {
            continue;
        };
        let bound = pairing
            .relays
            .iter()
            .any(|r| canonical_relay_url(r).is_ok_and(|r| r == relay));
        if !bound {
            continue;
        }
        match Provider::open(endpoint, pairing.token, true, relay_url).await {
            Ok(Ok(provider)) => return Ok(Some(provider)),
            Ok(Err(Skip::Revoked)) => {
                pairings.0.remove(&instance);
                revoked = true;
                eprintln!(
                    "warning: Buzz ({instance}) revoked this CLI's pairing; \
                     run `buzz auth desktop pair` to pair again"
                );
            }
            Ok(Err(Skip::NotApplicable)) => {}
            // An advertised desktop that cannot be confirmed or reached is
            // skipped: the chain moves on rather than failing every command.
            Err(_) => {}
        }
    }
    if revoked {
        let _ = save_pairings(&pairings);
    }
    Ok(None)
}

#[derive(serde::Deserialize)]
struct Paired {
    token: String,
    instance: String,
    pubkey: String,
    relays: Vec<String>,
}

/// Ask the running desktop to pair this CLI for `relay_url`, holding until the
/// person decides, and store the token in the OS keychain.
pub async fn pair(
    relay_url: &str,
    client_name: &str,
    instance: Option<&str>,
) -> Result<serde_json::Value, CliError> {
    let mut found = advertisements();
    if let Some(want) = instance {
        found.retain(|(i, _)| i == want);
    }
    let (instance, endpoint) = match found.len() {
        0 => {
            return Err(CliError::Auth(
                "no running Buzz desktop found; open Buzz and try again".into(),
            ))
        }
        1 => found.remove(0),
        _ => {
            let names: Vec<_> = found.iter().map(|(i, _)| i.as_str()).collect();
            return Err(CliError::Usage(format!(
                "several Buzz desktops are running ({}); pass --instance",
                names.join(", ")
            )));
        }
    };
    // Fail on a missing keychain before asking the person to approve.
    let mut pairings = load_pairings()?;
    let relay = canonical_relay_url(relay_url)?;
    let raw = call(
        &endpoint,
        None,
        true,
        "pair",
        &[client_name.to_owned(), relay.clone()],
    )
    .await
    .map_err(|e| e.into_cli("pair"))?;
    let paired: Paired = serde_json::from_str(&raw)
        .map_err(|_| CliError::Auth("Buzz desktop: unreadable `pair` answer".into()))?;
    if paired.instance != instance
        || !usable_token(&paired.token)
        || PublicKey::from_hex(&paired.pubkey).is_err()
    {
        return Err(CliError::Auth(
            "Buzz desktop returned an invalid pairing".into(),
        ));
    }
    let summary = serde_json::json!({
        "instance": instance,
        "pubkey": paired.pubkey,
        "relays": paired.relays,
    });
    // A new pairing for an instance replaces its old token.
    pairings.0.insert(
        instance,
        Pairing {
            token: paired.token,
            pubkey: paired.pubkey,
            relays: paired.relays,
        },
    );
    save_pairings(&pairings)?;
    Ok(summary)
}

/// Forget paired tokens: one instance's, or all of them. Local only.
pub fn unpair(instance: Option<&str>) -> Result<serde_json::Value, CliError> {
    let mut pairings = load_pairings()?;
    let removed: Vec<String> = match instance {
        Some(i) => pairings
            .0
            .remove(i)
            .map(|_| vec![i.to_owned()])
            .unwrap_or_default(),
        None => std::mem::take(&mut pairings.0).into_keys().collect(),
    };
    save_pairings(&pairings)?;
    Ok(serde_json::json!({ "removed": removed }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    #[test]
    fn endpoint_must_be_loopback_http_ipv4() {
        assert!(Endpoint::parse("http://127.0.0.1:4100/rpc").is_ok());
        for bad in [
            "https://127.0.0.1:4100/rpc",
            "http://localhost:4100/rpc",
            "http://[::1]:4100/rpc",
            "http://user@127.0.0.1:4100/rpc",
            "http://10.0.0.1:4100/rpc",
            "http://127.0.0.1/rpc",
        ] {
            assert!(Endpoint::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn unrecognized_codes_are_denied() {
        assert_eq!(known_code("approval_required"), "approval_required");
        assert_eq!(known_code("brand_new_code"), "denied");
    }

    #[test]
    fn held_methods_get_the_long_timeout() {
        assert!(held("sign_event") && held("nip44_decrypt") && held("pair"));
        assert!(!held("describe") && !held("federated_identity"));
    }

    #[test]
    fn parses_content_length_and_chunked_responses() {
        let fixed = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhiEXTRA";
        assert_eq!(parse_response(fixed), Some((200, b"hi".to_vec())));
        let chunked =
            b"HTTP/1.1 403 Forbidden\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n";
        assert_eq!(parse_response(chunked), Some((403, b"abcde".to_vec())));
        assert_eq!(parse_response(b"garbage"), None);
    }

    #[test]
    fn proc_net_tcp_finds_the_server_side_of_this_connection() {
        let table = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
           0: 0100007F:1004 00000000:0000 0A 00000000:00000000 00:00000000 00000000   501        0 1\n\
           1: 0100007F:1004 0100007F:C350 01 00000000:00000000 00:00000000 00000000   502        0 2\n\
           2: 0100007F:C350 0100007F:1004 01 00000000:00000000 00:00000000 00000000   501        0 3\n";
        assert_eq!(proc_net_tcp_uid(table, 0x1004, 0xC350), Some(502));
        assert_eq!(proc_net_tcp_uid(table, 0x1004, 0xC351), None);
    }

    #[test]
    fn lsof_finds_the_server_side_of_this_connection() {
        let out = "p100\nu501\nn127.0.0.1:50000->127.0.0.1:4100\np200\nu502\nn127.0.0.1:4100->127.0.0.1:50000\n";
        assert_eq!(lsof_uid(out, 4100, 50000), Some(502));
        assert_eq!(lsof_uid(out, 4100, 50001), None);
    }

    #[test]
    fn jwt_exp_reads_the_payload_claim() {
        use base64::Engine;
        let payload =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"exp":1790000300}"#);
        assert_eq!(jwt_exp(&format!("e30.{payload}.c2ln")), Some(1_790_000_300));
        assert_eq!(jwt_exp("e30.e30"), None);
        assert_eq!(jwt_exp("e30.e30.c2ln.x"), None);
    }

    #[test]
    fn advertisements_require_private_ownership() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let ads = dir.path().join("providers");
        std::fs::create_dir(&ads).unwrap();
        let write = |name: &str, mode: u32| {
            let path = ads.join(name);
            std::fs::write(
                &path,
                r#"{"instance":"xyz.block.buzz","url":"http://127.0.0.1:4100/rpc"}"#,
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        write("good.json", 0o600);
        write("open.json", 0o644);
        write(".partial.json", 0o600);
        std::fs::set_permissions(&ads, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            advertisements_in(&ads).is_empty(),
            "a group-readable directory is ignored"
        );
        std::fs::set_permissions(&ads, std::fs::Permissions::from_mode(0o700)).unwrap();
        let found = advertisements_in(&ads);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "xyz.block.buzz");
        assert_eq!(found[0].1.port, 4100);
    }

    // -----------------------------------------------------------------------
    // Against a mock provider
    // -----------------------------------------------------------------------

    type Seen = Arc<StdMutex<Vec<(Option<String>, String, serde_json::Value)>>>;

    /// A provider answering each envelope with `respond(version, request)`.
    async fn provider<F>(respond: F) -> (Endpoint, Seen)
    where
        F: Fn(&str, &serde_json::Value) -> serde_json::Value + Send + Sync + 'static,
    {
        use axum::http::HeaderMap;
        let seen: Seen = Arc::default();
        let log = seen.clone();
        let respond = Arc::new(respond);
        let app = axum::Router::new().route(
            "/rpc",
            axum::routing::post(move |headers: HeaderMap, body: axum::body::Bytes| {
                let (log, respond) = (log.clone(), respond.clone());
                async move {
                    let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                    let header = |n: &str| {
                        headers
                            .get(n)
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_owned)
                    };
                    let version = header(VERSION_HEADER).unwrap_or_default();
                    let mut answer = respond(&version, &request);
                    answer["id"] = request["id"].clone();
                    log.lock()
                        .unwrap()
                        .push((header("authorization"), version, request));
                    axum::Json(answer)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (
            Endpoint::parse(&format!("http://127.0.0.1:{port}/rpc")).unwrap(),
            seen,
        )
    }

    const RELAY: &str = "wss://community.example";

    fn describe(relays: &[&str]) -> serde_json::Value {
        let pubkey = nostr::Keys::generate().public_key().to_hex();
        serde_json::json!({"result": serde_json::json!({
            "instance": "xyz.block.buzz", "pubkey": pubkey, "role": "owner",
            "relays": relays, "methods": ["get_public_key", "sign_event"],
        }).to_string()})
    }

    #[tokio::test]
    async fn describe_sends_token_and_version_and_binds_the_relay() {
        let (endpoint, seen) = provider(|_, _| describe(&[RELAY])).await;
        let opened = Provider::open(endpoint, "tok".into(), false, RELAY)
            .await
            .unwrap();
        assert!(opened.is_ok());
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].0.as_deref(), Some("Bearer tok"));
        assert_eq!(seen[0].1, "1");
        assert_eq!(seen[0].2["method"], "describe");
    }

    #[tokio::test]
    async fn an_unbound_relay_or_revoked_token_falls_through() {
        let (endpoint, _) = provider(|_, _| describe(&["wss://other.example"])).await;
        let opened = Provider::open(endpoint, "tok".into(), false, RELAY)
            .await
            .unwrap();
        assert!(matches!(opened, Err(Skip::NotApplicable)));
        let (endpoint, _) =
            provider(|_, _| serde_json::json!({"result": "", "error": "revoked"})).await;
        let opened = Provider::open(endpoint, "tok".into(), false, RELAY)
            .await
            .unwrap();
        assert!(matches!(opened, Err(Skip::Revoked)));
    }

    #[tokio::test]
    async fn version_negotiation_retries_once_or_gives_up() {
        let (endpoint, seen) = provider(|_, _| {
            serde_json::json!({"result": "", "error": "unsupported_version", "versions": [2, 3]})
        })
        .await;
        let err = call(&endpoint, Some("tok"), false, "describe", &[])
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::NoCommonVersion));
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn an_unknown_error_code_is_denied() {
        let (endpoint, _) =
            provider(|_, _| serde_json::json!({"result": "", "error": "brand_new"})).await;
        let err = call(&endpoint, Some("tok"), false, "sign_event", &[])
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::Code { code: "denied", .. }));
    }

    #[tokio::test]
    async fn same_user_check_accepts_this_users_provider() {
        let (endpoint, _) = provider(|_, _| describe(&[RELAY])).await;
        let opened = Provider::open(endpoint, "tok".into(), true, RELAY)
            .await
            .unwrap();
        assert!(opened.is_ok());
    }
}
