//! Owner-only, read-only Buzz conversation operator.
//!
//! `buzz-read` is a credentialless Unix-socket client. The already-running
//! Buzz Desktop process owns the socket, resolves the active relay and signer
//! from `AppState`, performs the authenticated query, and returns a bounded
//! response. No private key or Authorization header crosses the process
//! boundary.

use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    fs,
    io::{Read, Write},
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
        net::UnixStream as StdUnixStream,
    },
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use chrono::DateTime;
use futures_util::StreamExt;
use nostr::{Event, Keys};
use regex::Regex;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use tauri::Manager;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    time::timeout,
};
use url::Url;

use crate::app_state::{AppState, IdentityStorage};

const SCHEMA_VERSION: u32 = 1;
const MAX_REQUEST_BYTES: usize = 16 * 1024;
const MAX_RELAY_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_RECEIPT_BYTES: usize = 128 * 1024;
const MAX_RESULTS: u32 = 100;
const MAX_EXCERPT_CHARS: u32 = 512;
const MAX_SEARCH_CHARS: usize = 256;
const MAX_RANGE_SECONDS: i64 = 31 * 24 * 60 * 60;
const MAX_REQUEST_LIFETIME_SECONDS: i64 = 30;
const MAX_CLOCK_SKEW_SECONDS: i64 = 5;
const SOCKET_IO_TIMEOUT: Duration = Duration::from_secs(45);
const SOCKET_DIR_NAME: &str = "operator-read";
const SOCKET_FILE_NAME: &str = "desktop.sock";
const ALLOWED_RELAY_HOST: &str = "buildcontext.communities.buzz.xyz";
const PRODUCTION_BUNDLE_IDENTIFIER: &str = "xyz.block.buzz.app";
#[cfg(target_os = "macos")]
const PRODUCTION_CODE_REQUIREMENT: &str = "identifier \"xyz.block.buzz.app\" and anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = EYF346PHUG";
const MESSAGE_KINDS: [u32; 4] = [9, 40002, 45001, 45003];

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReadRequest {
    schema_version: u32,
    request_id: String,
    operation: String,
    issued_at: i64,
    expires_at: i64,
    since: i64,
    until: i64,
    limit: u32,
    excerpt_chars: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_relay: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_identity_pubkey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    channel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    search: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ReadReceipt {
    schema_version: u32,
    request_id: String,
    status: String,
    operation: String,
    generated_at: i64,
    desktop_pid: u32,
    relay_host: String,
    identity_pubkey: String,
    requested_limit: u32,
    returned: usize,
    truncated: bool,
    events: Vec<ReceiptEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ReceiptEvent {
    id: String,
    author_pubkey: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    author_name: Option<String>,
    kind: u32,
    created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    channel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    excerpt: Option<String>,
}

#[derive(Debug)]
struct OperatorError {
    code: &'static str,
    message: &'static str,
}

impl OperatorError {
    const fn new(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
    }
}

#[derive(Default)]
struct ReplayGuard {
    consumed: Mutex<HashMap<String, i64>>,
}

struct ActiveScope {
    relay: String,
    keys: Keys,
    identity_pubkey: String,
    workspace_generation: u64,
}

impl ReplayGuard {
    fn consume(&self, request_id: &str, expires_at: i64, now: i64) -> Result<(), OperatorError> {
        let mut consumed = self.consumed.lock().map_err(|_| {
            OperatorError::new("service_unavailable", "the replay fence was unavailable")
        })?;
        consumed.retain(|_, expiry| *expiry >= now);
        if consumed.contains_key(request_id) {
            return Err(OperatorError::new(
                "request_replayed",
                "the read request was already consumed",
            ));
        }
        consumed.insert(request_id.to_string(), expires_at);
        Ok(())
    }
}

struct SocketCleanup {
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl Drop for SocketCleanup {
    fn drop(&mut self) {
        let Ok(metadata) = fs::symlink_metadata(&self.path) else {
            return;
        };
        if metadata.file_type().is_socket()
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Entry point for the credentialless `buzz-read` binary.
pub fn run_operator_read_cli<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let args = args.into_iter().collect::<Vec<_>>();
    match run_client_args(&args) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("buzz-read: {}", error.message);
            1
        }
    }
}

fn run_client_args(args: &[OsString]) -> Result<(), OperatorError> {
    reject_secret_environment()?;
    let now = unix_now()?;
    let request = parse_client_args(args, now)?;
    validate_request_at(&request, now)?;

    let socket_path = resolve_socket_path()?;
    validate_socket(&socket_path)?;
    let mut stream = StdUnixStream::connect(&socket_path).map_err(|_| {
        OperatorError::new(
            "app_unavailable",
            "the signed Buzz Desktop read service is not available",
        )
    })?;
    stream
        .set_read_timeout(Some(SOCKET_IO_TIMEOUT))
        .and_then(|_| stream.set_write_timeout(Some(SOCKET_IO_TIMEOUT)))
        .map_err(|_| {
            OperatorError::new(
                "app_unavailable",
                "could not configure the Buzz Desktop connection",
            )
        })?;

    let bytes = serde_json::to_vec(&request).map_err(|_| {
        OperatorError::new(
            "request_invalid",
            "the read request could not be serialized",
        )
    })?;
    write_frame_sync(&mut stream, &bytes, MAX_REQUEST_BYTES)?;
    let response = read_frame_sync(&mut stream, MAX_RECEIPT_BYTES)?;
    let receipt: ReadReceipt = serde_json::from_slice(&response).map_err(|_| {
        OperatorError::new("receipt_invalid", "the Buzz read receipt was malformed")
    })?;
    if receipt.request_id != request.request_id {
        return Err(OperatorError::new(
            "receipt_invalid",
            "the Buzz read receipt did not match the request",
        ));
    }
    let output = serde_json::to_vec_pretty(&receipt).map_err(|_| {
        OperatorError::new(
            "receipt_invalid",
            "could not serialize the Buzz read receipt",
        )
    })?;
    std::io::stdout()
        .write_all(&output)
        .and_then(|_| std::io::stdout().write_all(b"\n"))
        .map_err(|_| OperatorError::new("output_failed", "could not write the read receipt"))?;
    if receipt.status != "ok" {
        return Err(OperatorError::new(
            "read_failed",
            "Buzz Desktop could not complete the authenticated read",
        ));
    }
    Ok(())
}

fn parse_client_args(args: &[OsString], now: i64) -> Result<ReadRequest, OperatorError> {
    if args.len() < 2 || args[1].to_str() != Some("messages") {
        return Err(OperatorError::new(
            "usage",
            "usage: buzz-read messages --since <RFC3339|unix> --until <RFC3339|unix> [--channel <uuid>] [--search <text>] [--limit 1..100] [--excerpt-chars 0..512] [--expected-relay <wss-url>] [--expected-pubkey <hex>]",
        ));
    }

    let mut values: HashMap<&str, String> = HashMap::new();
    let mut index = 2;
    while index < args.len() {
        let flag = args[index]
            .to_str()
            .ok_or_else(|| OperatorError::new("usage", "arguments must be valid UTF-8"))?;
        let known = matches!(
            flag,
            "--since"
                | "--until"
                | "--channel"
                | "--search"
                | "--limit"
                | "--excerpt-chars"
                | "--expected-relay"
                | "--expected-pubkey"
        );
        if !known || values.contains_key(flag) || index + 1 >= args.len() {
            return Err(OperatorError::new(
                "usage",
                "the Buzz read arguments were invalid or duplicated",
            ));
        }
        let value = args[index + 1]
            .to_str()
            .ok_or_else(|| OperatorError::new("usage", "argument values must be valid UTF-8"))?;
        values.insert(flag, value.to_string());
        index += 2;
    }

    Ok(ReadRequest {
        schema_version: SCHEMA_VERSION,
        request_id: uuid::Uuid::new_v4().to_string(),
        operation: "messages".to_string(),
        issued_at: now,
        expires_at: now + MAX_REQUEST_LIFETIME_SECONDS,
        since: parse_time(&required_value(&values, "--since")?)?,
        until: parse_time(&required_value(&values, "--until")?)?,
        limit: optional_u32(&values, "--limit")?.unwrap_or(50),
        excerpt_chars: optional_u32(&values, "--excerpt-chars")?.unwrap_or(280),
        expected_relay: values.get("--expected-relay").cloned(),
        expected_identity_pubkey: values.get("--expected-pubkey").cloned(),
        channel: values.get("--channel").cloned(),
        search: values.get("--search").cloned(),
    })
}

fn required_value(values: &HashMap<&str, String>, name: &str) -> Result<String, OperatorError> {
    values
        .get(name)
        .cloned()
        .ok_or_else(|| OperatorError::new("usage", "since and until are required"))
}

fn optional_u32(values: &HashMap<&str, String>, name: &str) -> Result<Option<u32>, OperatorError> {
    values
        .get(name)
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|_| OperatorError::new("usage", "numeric options were invalid"))
        })
        .transpose()
}

fn parse_time(value: &str) -> Result<i64, OperatorError> {
    value.parse::<i64>().or_else(|_| {
        DateTime::parse_from_rfc3339(value)
            .map(|time| time.timestamp())
            .map_err(|_| OperatorError::new("usage", "time must be RFC3339 or Unix seconds"))
    })
}

fn validate_request_at(request: &ReadRequest, now: i64) -> Result<(), OperatorError> {
    if request.schema_version != SCHEMA_VERSION || request.operation != "messages" {
        return Err(OperatorError::new(
            "operation_rejected",
            "only the version-1 messages read operation is available",
        ));
    }
    uuid::Uuid::parse_str(&request.request_id)
        .map_err(|_| OperatorError::new("request_invalid", "request_id must be a UUID"))?;
    if request.issued_at > now + MAX_CLOCK_SKEW_SECONDS
        || request.expires_at <= now
        || request.expires_at <= request.issued_at
        || request.expires_at - request.issued_at > MAX_REQUEST_LIFETIME_SECONDS
    {
        return Err(OperatorError::new(
            "request_stale",
            "the read request was stale or had an invalid lifetime",
        ));
    }
    if request.since <= 0
        || request.until <= request.since
        || request.until - request.since > MAX_RANGE_SECONDS
    {
        return Err(OperatorError::new(
            "range_rejected",
            "the requested date range must be positive, ordered, and no longer than 31 days",
        ));
    }
    if request.limit == 0 || request.limit > MAX_RESULTS {
        return Err(OperatorError::new(
            "limit_rejected",
            "the requested limit must be between 1 and 100",
        ));
    }
    if request.excerpt_chars > MAX_EXCERPT_CHARS {
        return Err(OperatorError::new(
            "limit_rejected",
            "excerpt_chars must be between 0 and 512",
        ));
    }
    if let Some(relay) = request.expected_relay.as_deref() {
        validate_relay(relay)?;
    }
    if let Some(pubkey) = request.expected_identity_pubkey.as_deref() {
        if pubkey.len() != 64 || !pubkey.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(OperatorError::new(
                "identity_rejected",
                "expected-pubkey must be a 64-character hexadecimal public key",
            ));
        }
    }
    if let Some(channel) = request.channel.as_deref() {
        uuid::Uuid::parse_str(channel)
            .map_err(|_| OperatorError::new("channel_rejected", "channel must be a UUID"))?;
    }
    if let Some(search) = request.search.as_deref() {
        if search.trim().is_empty()
            || search.chars().count() > MAX_SEARCH_CHARS
            || search.chars().any(char::is_control)
        {
            return Err(OperatorError::new(
                "search_rejected",
                "search text must be non-empty, printable, and at most 256 characters",
            ));
        }
    }
    Ok(())
}

fn ensure_request_not_expired_at(expires_at: i64, now: i64) -> Result<(), OperatorError> {
    if expires_at <= now {
        return Err(OperatorError::new(
            "request_stale",
            "the read request expired before relay authentication",
        ));
    }
    Ok(())
}

fn validate_relay(value: &str) -> Result<Url, OperatorError> {
    let parsed = Url::parse(value)
        .map_err(|_| OperatorError::new("relay_rejected", "relay must be a valid URL"))?;
    if !matches!(parsed.scheme(), "wss" | "https")
        || parsed.host_str() != Some(ALLOWED_RELAY_HOST)
        || parsed.port().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
        || parsed.query().is_some()
        || !matches!(parsed.path(), "" | "/")
    {
        return Err(OperatorError::new(
            "relay_rejected",
            "the active relay must be the canonical Buzz wss or https origin",
        ));
    }
    Ok(parsed)
}

/// Start the owner-only Unix-socket service inside the signed Desktop process.
pub fn start_operator_read_server(app: tauri::AppHandle) -> Result<(), String> {
    if !is_production_bundle(&app) {
        return Ok(());
    }
    let state = app.state::<AppState>();
    if capture_production_keys(&state).is_err() {
        return Err(
            "operator reads require Block's signed production app and keyring-backed identity"
                .to_string(),
        );
    }
    let socket_path = resolve_socket_path().map_err(|error| error.message.to_string())?;
    prepare_socket_path(&socket_path).map_err(|error| error.message.to_string())?;
    let listener = UnixListener::bind(&socket_path)
        .map_err(|error| format!("could not bind owner-only socket: {error}"))?;
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("could not protect owner-only socket: {error}"))?;
    let metadata = validate_socket(&socket_path).map_err(|error| error.message.to_string())?;
    let cleanup = SocketCleanup {
        path: socket_path,
        device: metadata.dev(),
        inode: metadata.ino(),
    };
    let replay_guard = Arc::new(ReplayGuard::default());

    tauri::async_runtime::spawn(async move {
        let _cleanup = cleanup;
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                eprintln!("buzz-desktop: operator read service stopped accepting requests");
                break;
            };
            let app = app.clone();
            let replay_guard = replay_guard.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = handle_connection(stream, app, replay_guard).await {
                    eprintln!("buzz-desktop: operator read request failed: {}", error.code);
                }
            });
        }
    });
    Ok(())
}

pub fn is_production_bundle(app: &tauri::AppHandle) -> bool {
    app.config().identifier == PRODUCTION_BUNDLE_IDENTIFIER
}

/// Return whether this process is the trusted production credential owner.
pub fn is_trusted_production_owner(app: &tauri::AppHandle) -> bool {
    let state = app.state::<AppState>();
    is_production_bundle(app) && capture_production_keys(&state).is_ok()
}

fn production_credential_owner_allowed(
    identifier: &str,
    storage: IdentityStorage,
    code_signature_valid: bool,
) -> bool {
    if identifier != PRODUCTION_BUNDLE_IDENTIFIER || !code_signature_valid {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        storage == IdentityStorage::SystemKeyring
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = storage;
        false
    }
}

#[cfg(target_os = "macos")]
fn production_code_signature_valid() -> bool {
    use security_framework::os::macos::code_signing::{Flags, SecCode, SecRequirement};

    let Ok(requirement) = PRODUCTION_CODE_REQUIREMENT.parse::<SecRequirement>() else {
        return false;
    };
    let Ok(code) = SecCode::for_self(Flags::NONE) else {
        return false;
    };
    code.check_validity(
        Flags::STRICT_VALIDATE | Flags::CHECK_TRUSTED_ANCHORS | Flags::NO_NETWORK_ACCESS,
        &requirement,
    )
    .is_ok()
}

#[cfg(not(target_os = "macos"))]
fn production_code_signature_valid() -> bool {
    false
}

/// Expose the bundled credentialless client on the normal local PATH.
///
/// Development bundles deliberately do not create this production command.
/// Existing regular files are preserved; only a symlink in Buzz's own
/// `buzz-read` namespace is refreshed on application boot.
pub fn ensure_client_symlink(exe_parent: &Path) -> Result<(), String> {
    let local_bin = dirs::home_dir()
        .ok_or("cannot resolve home directory")?
        .join(".local")
        .join("bin");
    ensure_client_symlink_at(exe_parent, &local_bin)
}

fn ensure_client_symlink_at(exe_parent: &Path, local_bin: &Path) -> Result<(), String> {
    let bundled = exe_parent.join("buzz-read");
    if !bundled.is_file() || bundled.is_symlink() {
        return Ok(());
    }
    fs::create_dir_all(local_bin).map_err(|error| {
        format!(
            "create Buzz read client directory {}: {error}",
            local_bin.display()
        )
    })?;
    let link = local_bin.join("buzz-read");
    match link.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            fs::remove_file(&link)
                .map_err(|error| format!("remove stale {}: {error}", link.display()))?;
        }
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("stat {}: {error}", link.display())),
    }
    std::os::unix::fs::symlink(&bundled, &link)
        .map_err(|error| format!("symlink {}: {error}", link.display()))
}

async fn handle_connection(
    mut stream: UnixStream,
    app: tauri::AppHandle,
    replay_guard: Arc<ReplayGuard>,
) -> Result<(), OperatorError> {
    validate_peer(&stream)?;

    let raw = timeout(
        SOCKET_IO_TIMEOUT,
        read_frame_async(&mut stream, MAX_REQUEST_BYTES),
    )
    .await
    .map_err(|_| OperatorError::new("request_timeout", "the read request timed out"))??;
    let parsed = serde_json::from_slice::<ReadRequest>(&raw);
    let request_id = parsed
        .as_ref()
        .map(|request| request.request_id.clone())
        .unwrap_or_else(|_| "unknown".to_string());
    let request_expires_at = parsed.as_ref().ok().map(|request| request.expires_at);
    let result = async {
        let request = parsed
            .map_err(|_| OperatorError::new("request_invalid", "the read request was malformed"))?;
        let now = unix_now()?;
        validate_request_at(&request, now)?;
        replay_guard.consume(&request.request_id, request.expires_at, now)?;
        let remaining = duration_until_expiry(request.expires_at)?;
        run_with_expiry_timeout(remaining, execute_read(&app, request)).await
    }
    .await;
    let receipt = result.unwrap_or_else(|error| error_receipt(request_id, &error));
    ensure_receipt_bound(&receipt)?;
    let encoded = serde_json::to_vec(&receipt).map_err(|_| {
        OperatorError::new(
            "receipt_invalid",
            "the Buzz read receipt could not be serialized",
        )
    })?;
    let response_timeout = match request_expires_at {
        Some(expires_at) => duration_until_expiry(expires_at)?.min(SOCKET_IO_TIMEOUT),
        None => SOCKET_IO_TIMEOUT,
    };
    timeout(
        response_timeout,
        write_frame_async(&mut stream, &encoded, MAX_RECEIPT_BYTES),
    )
    .await
    .map_err(|_| OperatorError::new("response_timeout", "the read response timed out"))??;
    Ok(())
}

fn validate_peer(stream: &UnixStream) -> Result<(), OperatorError> {
    let credentials = stream
        .peer_cred()
        .map_err(|_| OperatorError::new("peer_rejected", "could not verify the local caller"))?;
    ensure_peer_uid(credentials.uid(), current_uid())
}

fn ensure_peer_uid(peer_uid: u32, owner_uid: u32) -> Result<(), OperatorError> {
    if peer_uid != owner_uid {
        return Err(OperatorError::new(
            "peer_rejected",
            "the local caller did not match the Buzz Desktop owner",
        ));
    }
    Ok(())
}

async fn execute_read(
    app: &tauri::AppHandle,
    request: ReadRequest,
) -> Result<ReadReceipt, OperatorError> {
    let state = app.state::<AppState>();
    let scope = capture_active_scope(&state).await?;
    if let Some(expected) = request.expected_relay.as_deref() {
        if crate::relay::relay_http_base_url(expected)
            != crate::relay::relay_http_base_url(&scope.relay)
        {
            return Err(OperatorError::new(
                "relay_mismatch",
                "the active Buzz relay did not match the expected relay",
            ));
        }
    }
    if request
        .expected_identity_pubkey
        .as_deref()
        .is_some_and(|expected| !expected.eq_ignore_ascii_case(&scope.identity_pubkey))
    {
        return Err(OperatorError::new(
            "identity_mismatch",
            "the active Buzz identity did not match the expected public key",
        ));
    }

    let mut filter = serde_json::json!({
        "kinds": MESSAGE_KINDS,
        "since": request.since,
        "until": request.until,
        "limit": request.limit.saturating_add(1),
    });
    if let Some(channel) = request.channel.as_deref() {
        filter["#h"] = serde_json::json!([channel]);
    }
    if let Some(search) = request.search.as_deref() {
        filter["search"] = serde_json::json!(search.trim());
    }

    let api_base = crate::relay::relay_http_base_url(&scope.relay);
    let mut events = query_verified(
        &state,
        &api_base,
        &[filter],
        &scope.keys,
        request.expires_at,
    )
    .await?;
    assert_active_scope_unchanged(&state, &scope).await?;
    if events.len() > request.limit.saturating_add(1) as usize {
        return Err(OperatorError::new(
            "response_oversize",
            "the Buzz relay returned more events than requested",
        ));
    }
    events.retain(|event| event_matches_request(event, &request));
    events.sort_by(|left, right| {
        right
            .created_at
            .as_secs()
            .cmp(&left.created_at.as_secs())
            .then_with(|| left.id.to_hex().cmp(&right.id.to_hex()))
    });
    let mut seen = HashSet::new();
    events.retain(|event| seen.insert(event.id.to_hex()));
    let truncated = events.len() > request.limit as usize;
    events.truncate(request.limit as usize);

    let author_names =
        fetch_author_names(&state, &api_base, &scope.keys, &events, request.expires_at)
            .await
            .unwrap_or_default();
    assert_active_scope_unchanged(&state, &scope).await?;
    ensure_request_not_expired_at(request.expires_at, unix_now()?)?;
    let projected = events
        .iter()
        .map(|event| project_event(event, request.excerpt_chars, &author_names))
        .collect::<Vec<_>>();
    let receipt = ReadReceipt {
        schema_version: SCHEMA_VERSION,
        request_id: request.request_id,
        status: "ok".to_string(),
        operation: "messages".to_string(),
        generated_at: unix_now()?,
        desktop_pid: std::process::id(),
        relay_host: ALLOWED_RELAY_HOST.to_string(),
        identity_pubkey: scope.identity_pubkey,
        requested_limit: request.limit,
        returned: projected.len(),
        truncated,
        events: projected,
        error_code: None,
        message: None,
    };
    ensure_receipt_bound(&receipt)?;
    Ok(receipt)
}

async fn capture_active_scope(state: &AppState) -> Result<ActiveScope, OperatorError> {
    let _workspace_guard = state.workspace_apply_lock.lock().await;
    let workspace_generation = state.workspace_apply_generation.load(Ordering::Acquire);
    let relay = crate::relay::relay_ws_url_with_override(state);
    validate_relay(&relay)?;
    let keys = capture_production_keys(state)?;
    let identity_pubkey = keys.public_key().to_hex();
    Ok(ActiveScope {
        relay,
        keys,
        identity_pubkey,
        workspace_generation,
    })
}

async fn assert_active_scope_unchanged(
    state: &AppState,
    initial: &ActiveScope,
) -> Result<(), OperatorError> {
    let _workspace_guard = state.workspace_apply_lock.lock().await;
    let current_generation = state.workspace_apply_generation.load(Ordering::Acquire);
    let current_relay = crate::relay::relay_ws_url_with_override(state);
    let current_pubkey = capture_production_keys(state)?.public_key().to_hex();
    ensure_scope_values_unchanged(
        initial.workspace_generation,
        &initial.relay,
        &initial.identity_pubkey,
        current_generation,
        &current_relay,
        &current_pubkey,
    )
}

fn capture_production_keys(state: &AppState) -> Result<Keys, OperatorError> {
    capture_production_keys_with_signature(state, production_code_signature_valid())
}

fn capture_production_keys_with_signature(
    state: &AppState,
    code_signature_valid: bool,
) -> Result<Keys, OperatorError> {
    // Identity writers change both the keys and their storage classification
    // while holding this mutex. Read the classification only after acquiring
    // the same mutex so a same-key keyring-to-file import cannot slip between
    // an owner check and the cloned signing keys.
    let keys = state.keys.lock().map_err(|_| {
        OperatorError::new(
            "identity_unavailable",
            "the active Buzz Desktop identity was unavailable",
        )
    })?;
    let recovery_mode =
        state.identity_lost.load(Ordering::Acquire) || state.keyring_locked.load(Ordering::Acquire);
    if recovery_mode
        || !production_credential_owner_allowed(
            PRODUCTION_BUNDLE_IDENTIFIER,
            state.identity_storage(),
            code_signature_valid,
        )
    {
        return Err(OperatorError::new(
            "identity_unavailable",
            "the active identity is no longer owned by Block's signed production app keyring",
        ));
    }
    Ok(keys.clone())
}

fn ensure_scope_values_unchanged(
    initial_generation: u64,
    initial_relay: &str,
    initial_pubkey: &str,
    current_generation: u64,
    current_relay: &str,
    current_pubkey: &str,
) -> Result<(), OperatorError> {
    if initial_generation != current_generation
        || initial_relay != current_relay
        || initial_pubkey != current_pubkey
    {
        return Err(OperatorError::new(
            "active_scope_changed",
            "the active Buzz workspace changed during the read",
        ));
    }
    Ok(())
}

async fn query_verified(
    state: &AppState,
    api_base: &str,
    filters: &[serde_json::Value],
    keys: &Keys,
    expires_at: i64,
) -> Result<Vec<Event>, OperatorError> {
    crate::relay_admission::wait_for_rate_limit().await;
    ensure_request_not_expired_at(expires_at, unix_now()?)?;
    let url = format!("{}/query", api_base.trim_end_matches('/'));
    let body = serde_json::to_vec(filters).map_err(|_| {
        OperatorError::new(
            "request_invalid",
            "the Buzz relay filter could not be serialized",
        )
    })?;
    let authorization =
        crate::relay::build_nip98_auth_header_for_keys(keys, &Method::POST, &url, &body).map_err(
            |_| {
                OperatorError::new(
                    "identity_unavailable",
                    "Buzz Desktop could not authenticate the read",
                )
            },
        )?;
    let response = state
        .media_fetch_client
        .post(url)
        .header("Authorization", authorization)
        .header("Content-Type", "application/json")
        .timeout(Duration::from_secs(30))
        .body(body)
        .send()
        .await
        .map_err(|_| {
            OperatorError::new(
                "relay_unavailable",
                "the Buzz relay could not complete the authenticated read",
            )
        })?;
    if !response.status().is_success() {
        return Err(OperatorError::new(
            "relay_rejected",
            "the Buzz relay rejected the authenticated read",
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RELAY_RESPONSE_BYTES as u64)
    {
        return Err(OperatorError::new(
            "response_oversize",
            "the Buzz relay response exceeded the read bound",
        ));
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| {
            OperatorError::new(
                "relay_unavailable",
                "the Buzz relay response was interrupted",
            )
        })?;
        if bytes.len().saturating_add(chunk.len()) > MAX_RELAY_RESPONSE_BYTES {
            return Err(OperatorError::new(
                "response_oversize",
                "the Buzz relay response exceeded the read bound",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let events: Vec<Event> = serde_json::from_slice(&bytes).map_err(|_| {
        OperatorError::new("response_invalid", "the Buzz relay response was malformed")
    })?;
    verify_event_set(&events)?;
    ensure_request_not_expired_at(expires_at, unix_now()?)?;
    Ok(events)
}

fn verify_event_set(events: &[Event]) -> Result<(), OperatorError> {
    for event in events {
        event.verify().map_err(|_| {
            OperatorError::new(
                "response_unverified",
                "the Buzz relay returned an event that failed verification",
            )
        })?;
    }
    Ok(())
}

async fn fetch_author_names(
    state: &AppState,
    api_base: &str,
    keys: &Keys,
    events: &[Event],
    expires_at: i64,
) -> Result<HashMap<String, String>, OperatorError> {
    let authors = events
        .iter()
        .map(|event| event.pubkey.to_hex())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if authors.is_empty() {
        return Ok(HashMap::new());
    }
    let filter = serde_json::json!({
        "kinds": [0],
        "authors": authors,
        "limit": authors.len().min(MAX_RESULTS as usize),
    });
    let profiles = query_verified(state, api_base, &[filter], keys, expires_at).await?;
    if profiles.len() > MAX_RESULTS as usize {
        return Err(OperatorError::new(
            "response_oversize",
            "the Buzz relay returned too many author profiles",
        ));
    }
    let mut names = HashMap::new();
    for profile in profiles {
        let pubkey = profile.pubkey.to_hex();
        if names.contains_key(&pubkey) {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&profile.content) else {
            continue;
        };
        let name = value
            .get("display_name")
            .or_else(|| value.get("name"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty() && name.chars().count() <= 80)
            .map(redact_sensitive_text);
        if let Some(name) = name {
            names.insert(pubkey, name);
        }
    }
    Ok(names)
}

fn event_matches_request(event: &Event, request: &ReadRequest) -> bool {
    let kind = event.kind.as_u16() as u32;
    let created_at = event.created_at.as_secs() as i64;
    MESSAGE_KINDS.contains(&kind)
        && created_at >= request.since
        && created_at <= request.until
        && request
            .channel
            .as_deref()
            .is_none_or(|expected| event_channel(event).as_deref() == Some(expected))
        && request.search.as_deref().is_none_or(|search| {
            event
                .content
                .to_lowercase()
                .contains(&search.trim().to_lowercase())
        })
}

fn project_event(
    event: &Event,
    excerpt_chars: u32,
    author_names: &HashMap<String, String>,
) -> ReceiptEvent {
    let author_pubkey = event.pubkey.to_hex();
    let excerpt = (excerpt_chars > 0).then(|| bounded_excerpt(&event.content, excerpt_chars));
    ReceiptEvent {
        id: event.id.to_hex(),
        author_name: author_names.get(&author_pubkey).cloned(),
        author_pubkey,
        kind: event.kind.as_u16() as u32,
        created_at: event.created_at.as_secs() as i64,
        channel: event_channel(event),
        excerpt,
    }
}

fn event_channel(event: &Event) -> Option<String> {
    event.tags.iter().find_map(|tag| {
        let values = tag.as_slice();
        if values.first().map(String::as_str) != Some("h") {
            return None;
        }
        let channel = values.get(1)?;
        uuid::Uuid::parse_str(channel).ok().map(|_| channel.clone())
    })
}

fn bounded_excerpt(content: &str, max_chars: u32) -> String {
    let normalized = content
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    let redacted = redact_sensitive_text(&normalized);
    let maximum = max_chars as usize;
    if redacted.chars().count() <= maximum {
        return redacted;
    }
    if maximum == 0 {
        return String::new();
    }
    let mut excerpt = redacted.chars().take(maximum - 1).collect::<String>();
    excerpt.push('…');
    excerpt
}

fn redact_sensitive_text(input: &str) -> String {
    static PATTERNS: std::sync::OnceLock<Vec<(Regex, &'static str)>> = std::sync::OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        vec![
            (
                Regex::new(r"(?i)nsec1[023456789acdefghjklmnpqrstuvwxyz]{16,}")
                    .expect("static nsec regex"),
                "[REDACTED_NSEC]",
            ),
            (
                Regex::new(r"(?i)(?:sk-|ghp_|github_pat_|xox[baprs]-)[A-Za-z0-9_-]{10,}")
                    .expect("static token regex"),
                "[REDACTED_TOKEN]",
            ),
            (
                Regex::new(r"(?i)(bearer\s+)[^\s,;]+")
                    .expect("static bearer regex"),
                "$1[REDACTED]",
            ),
            (
                Regex::new(r"(?i)((?:buzz_private_key|buzz_auth_tag|authorization|password|api[_ -]?key|token|secret)\s*[:=]\s*)[^\s,;]+")
                    .expect("static assignment regex"),
                "$1[REDACTED]",
            ),
        ]
    });
    patterns
        .iter()
        .fold(input.to_string(), |current, (regex, replacement)| {
            regex.replace_all(&current, *replacement).to_string()
        })
}

fn error_receipt(request_id: String, error: &OperatorError) -> ReadReceipt {
    ReadReceipt {
        schema_version: SCHEMA_VERSION,
        request_id,
        status: "error".to_string(),
        operation: "messages".to_string(),
        generated_at: unix_now().unwrap_or(0),
        desktop_pid: std::process::id(),
        relay_host: String::new(),
        identity_pubkey: String::new(),
        requested_limit: 0,
        returned: 0,
        truncated: false,
        events: Vec::new(),
        error_code: Some(error.code.to_string()),
        message: Some(error.message.to_string()),
    }
}

fn ensure_receipt_bound(receipt: &ReadReceipt) -> Result<(), OperatorError> {
    let bytes = serde_json::to_vec(receipt).map_err(|_| {
        OperatorError::new(
            "receipt_invalid",
            "the Buzz read receipt could not be serialized",
        )
    })?;
    if bytes.len() > MAX_RECEIPT_BYTES {
        return Err(OperatorError::new(
            "receipt_oversize",
            "the Buzz read receipt exceeded its output bound",
        ));
    }
    Ok(())
}

fn reject_secret_environment() -> Result<(), OperatorError> {
    for name in ["BUZZ_PRIVATE_KEY", "BUZZ_AUTH_TAG", "NOSTR_PRIVATE_KEY"] {
        if std::env::var_os(name).is_some() {
            return Err(OperatorError::new(
                "secret_input_rejected",
                "buzz-read refuses credential-bearing environment variables",
            ));
        }
    }
    Ok(())
}

fn resolve_socket_path() -> Result<PathBuf, OperatorError> {
    let buzz_dir = crate::managed_agents::nest_dir().ok_or_else(|| {
        OperatorError::new(
            "home_unavailable",
            "could not resolve the current Buzz application directory",
        )
    })?;
    validate_parent_dir(&buzz_dir)?;
    let socket_dir = buzz_dir.join(SOCKET_DIR_NAME);
    ensure_owner_only_dir(&socket_dir)?;
    Ok(socket_dir.join(SOCKET_FILE_NAME))
}

fn validate_parent_dir(path: &Path) -> Result<(), OperatorError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        OperatorError::new(
            "control_dir_unavailable",
            "the Buzz application directory was unavailable",
        )
    })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata.uid() != current_uid() {
        return Err(OperatorError::new(
            "control_dir_rejected",
            "the Buzz application directory failed type or owner checks",
        ));
    }
    Ok(())
}

fn ensure_owner_only_dir(path: &Path) -> Result<(), OperatorError> {
    if !path.exists() {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(false).mode(0o700);
        builder.create(path).map_err(|_| {
            OperatorError::new(
                "control_dir_unavailable",
                "could not create the owner-only Buzz read directory",
            )
        })?;
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        OperatorError::new(
            "control_dir_unavailable",
            "could not inspect the owner-only Buzz read directory",
        )
    })?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != current_uid()
        || metadata.permissions().mode() & 0o777 != 0o700
    {
        return Err(OperatorError::new(
            "control_dir_rejected",
            "the Buzz read directory must be a real owner-only directory",
        ));
    }
    Ok(())
}

fn prepare_socket_path(path: &Path) -> Result<(), OperatorError> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if !metadata.file_type().is_socket()
        || metadata.uid() != current_uid()
        || metadata.permissions().mode() & 0o777 != 0o600
    {
        return Err(OperatorError::new(
            "socket_rejected",
            "the Buzz read socket path was replaced by an unexpected object",
        ));
    }
    if StdUnixStream::connect(path).is_ok() {
        return Err(OperatorError::new(
            "socket_active",
            "another Buzz Desktop read service is already active",
        ));
    }
    let current = fs::symlink_metadata(path).map_err(|_| {
        OperatorError::new(
            "socket_rejected",
            "the stale Buzz read socket changed before cleanup",
        )
    })?;
    if !same_socket_identity(&metadata, &current) {
        return Err(OperatorError::new(
            "socket_rejected",
            "the stale Buzz read socket changed before cleanup",
        ));
    }
    fs::remove_file(path).map_err(|_| {
        OperatorError::new(
            "socket_rejected",
            "could not remove the stale Buzz read socket",
        )
    })
}

fn same_socket_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.file_type().is_socket()
        && right.file_type().is_socket()
        && left.uid() == right.uid()
        && left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.permissions().mode() & 0o777 == right.permissions().mode() & 0o777
}

fn validate_socket(path: &Path) -> Result<fs::Metadata, OperatorError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        OperatorError::new(
            "app_unavailable",
            "the signed Buzz Desktop read service is not available",
        )
    })?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != current_uid()
        || metadata.permissions().mode() & 0o777 != 0o600
    {
        return Err(OperatorError::new(
            "socket_rejected",
            "the Buzz read socket failed type, owner, or mode checks",
        ));
    }
    Ok(metadata)
}

fn write_frame_sync(
    stream: &mut StdUnixStream,
    payload: &[u8],
    maximum: usize,
) -> Result<(), OperatorError> {
    if payload.len() > maximum || payload.len() > u32::MAX as usize {
        return Err(OperatorError::new(
            "request_oversize",
            "the Buzz read frame exceeded its size bound",
        ));
    }
    stream
        .write_all(&(payload.len() as u32).to_be_bytes())
        .and_then(|_| stream.write_all(payload))
        .map_err(|_| {
            OperatorError::new(
                "app_unavailable",
                "could not send the request to Buzz Desktop",
            )
        })
}

fn read_frame_sync(stream: &mut StdUnixStream, maximum: usize) -> Result<Vec<u8>, OperatorError> {
    let mut header = [0_u8; 4];
    stream.read_exact(&mut header).map_err(|_| {
        OperatorError::new(
            "app_unavailable",
            "could not read the response from Buzz Desktop",
        )
    })?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > maximum {
        return Err(OperatorError::new(
            "receipt_oversize",
            "the Buzz read response exceeded its size bound",
        ));
    }
    let mut payload = vec![0_u8; length];
    stream.read_exact(&mut payload).map_err(|_| {
        OperatorError::new(
            "app_unavailable",
            "the Buzz Desktop response was interrupted",
        )
    })?;
    Ok(payload)
}

async fn read_frame_async(
    stream: &mut UnixStream,
    maximum: usize,
) -> Result<Vec<u8>, OperatorError> {
    let mut header = [0_u8; 4];
    stream.read_exact(&mut header).await.map_err(|_| {
        OperatorError::new("request_invalid", "the Buzz read request was interrupted")
    })?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > maximum {
        return Err(OperatorError::new(
            "request_oversize",
            "the Buzz read request exceeded its input bound",
        ));
    }
    let mut payload = vec![0_u8; length];
    stream.read_exact(&mut payload).await.map_err(|_| {
        OperatorError::new("request_invalid", "the Buzz read request was interrupted")
    })?;
    Ok(payload)
}

async fn write_frame_async(
    stream: &mut UnixStream,
    payload: &[u8],
    maximum: usize,
) -> Result<(), OperatorError> {
    if payload.len() > maximum || payload.len() > u32::MAX as usize {
        return Err(OperatorError::new(
            "receipt_oversize",
            "the Buzz read receipt exceeded its output bound",
        ));
    }
    stream
        .write_all(&(payload.len() as u32).to_be_bytes())
        .await
        .map_err(|_| {
            OperatorError::new(
                "response_interrupted",
                "could not return the Buzz read receipt",
            )
        })?;
    stream.write_all(payload).await.map_err(|_| {
        OperatorError::new(
            "response_interrupted",
            "could not return the Buzz read receipt",
        )
    })
}

fn unix_now() -> Result<i64, OperatorError> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| OperatorError::new("clock_invalid", "the system clock was invalid"))?
        .as_secs();
    i64::try_from(seconds)
        .map_err(|_| OperatorError::new("clock_invalid", "the system clock was invalid"))
}

fn duration_until_expiry(expires_at: i64) -> Result<Duration, OperatorError> {
    let seconds = u64::try_from(expires_at).map_err(|_| {
        OperatorError::new("request_stale", "the read request expired before execution")
    })?;
    let deadline = UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .ok_or_else(|| {
            OperatorError::new("request_stale", "the read request expiry was invalid")
        })?;
    deadline.duration_since(SystemTime::now()).map_err(|_| {
        OperatorError::new("request_stale", "the read request expired before execution")
    })
}

async fn run_with_expiry_timeout<F, T>(
    remaining: Duration,
    operation: F,
) -> Result<T, OperatorError>
where
    F: std::future::Future<Output = Result<T, OperatorError>>,
{
    timeout(remaining, operation).await.map_err(|_| {
        OperatorError::new("request_stale", "the read request expired during execution")
    })?
}

fn current_uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Kind, Tag};
    use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt};

    fn request(now: i64) -> ReadRequest {
        ReadRequest {
            schema_version: SCHEMA_VERSION,
            request_id: uuid::Uuid::new_v4().to_string(),
            operation: "messages".to_string(),
            issued_at: now,
            expires_at: now + MAX_REQUEST_LIFETIME_SECONDS,
            since: 1_787_872_400,
            until: 1_787_958_800,
            limit: 20,
            excerpt_chars: 280,
            expected_relay: None,
            expected_identity_pubkey: None,
            channel: None,
            search: None,
        }
    }

    #[test]
    fn request_allows_only_messages_and_denies_unknown_fields() {
        let now = 1_788_000_000;
        let mut candidate = request(now);
        candidate.operation = "send".to_string();
        assert_eq!(
            validate_request_at(&candidate, now).unwrap_err().code,
            "operation_rejected"
        );

        let mut value = serde_json::to_value(request(now)).unwrap();
        value["content"] = serde_json::json!("write this");
        assert!(serde_json::from_value::<ReadRequest>(value).is_err());
    }

    #[test]
    fn production_operator_rejects_nonproduction_and_nonkeyring_macos_owners() {
        assert!(!production_credential_owner_allowed(
            "xyz.contextdotbuild.buzz.client",
            IdentityStorage::SystemKeyring,
            true,
        ));
        assert!(!production_credential_owner_allowed(
            PRODUCTION_BUNDLE_IDENTIFIER,
            IdentityStorage::SystemKeyring,
            false,
        ));
        #[cfg(target_os = "macos")]
        {
            assert!(!production_credential_owner_allowed(
                PRODUCTION_BUNDLE_IDENTIFIER,
                IdentityStorage::Environment,
                true,
            ));
            assert!(!production_credential_owner_allowed(
                PRODUCTION_BUNDLE_IDENTIFIER,
                IdentityStorage::LocalFile,
                true,
            ));
            assert!(production_credential_owner_allowed(
                PRODUCTION_BUNDLE_IDENTIFIER,
                IdentityStorage::SystemKeyring,
                true,
            ));
            assert!(PRODUCTION_CODE_REQUIREMENT.contains("anchor apple generic"));
            assert!(PRODUCTION_CODE_REQUIREMENT.contains("EYF346PHUG"));
            use security_framework::os::macos::code_signing::SecRequirement;
            assert!(PRODUCTION_CODE_REQUIREMENT
                .parse::<SecRequirement>()
                .is_ok());
        }
    }

    #[test]
    fn same_key_storage_downgrade_cannot_race_the_owner_snapshot() {
        use std::sync::{mpsc, Barrier};

        let state = Arc::new(crate::app_state::build_app_state());
        state.set_identity_storage(IdentityStorage::SystemKeyring);
        let original_pubkey = state.keys.lock().unwrap().public_key();
        let writer_state = state.clone();
        let writer_ready = Arc::new(Barrier::new(2));
        let release_writer = Arc::new(Barrier::new(2));
        let writer_ready_thread = writer_ready.clone();
        let release_writer_thread = release_writer.clone();
        let writer = std::thread::spawn(move || {
            let keys = writer_state.keys.lock().unwrap();
            assert_eq!(keys.public_key(), original_pubkey);
            writer_state.set_identity_storage(IdentityStorage::LocalFile);
            writer_ready_thread.wait();
            release_writer_thread.wait();
        });
        writer_ready.wait();

        let reader_state = state.clone();
        let (result_tx, result_rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            result_tx
                .send(capture_production_keys_with_signature(&reader_state, true))
                .unwrap();
        });
        assert!(result_rx.recv_timeout(Duration::from_millis(50)).is_err());
        release_writer.wait();

        assert_eq!(
            result_rx.recv().unwrap().unwrap_err().code,
            "identity_unavailable"
        );
        writer.join().unwrap();
        reader.join().unwrap();
    }

    #[test]
    fn request_freshness_fails_closed() {
        let now = 1_788_000_000;
        let mut stale = request(now);
        stale.issued_at = now - 60;
        stale.expires_at = now - 30;
        assert_eq!(
            validate_request_at(&stale, now).unwrap_err().code,
            "request_stale"
        );

        let mut future = request(now);
        future.issued_at = now + MAX_CLOCK_SKEW_SECONDS + 1;
        future.expires_at = future.issued_at + 1;
        assert_eq!(
            validate_request_at(&future, now).unwrap_err().code,
            "request_stale"
        );
        assert_eq!(
            ensure_request_not_expired_at(now, now).unwrap_err().code,
            "request_stale"
        );
    }

    #[test]
    fn replay_guard_consumes_each_request_once() {
        let guard = ReplayGuard::default();
        guard.consume("one", 120, 100).unwrap();
        assert_eq!(
            guard.consume("one", 120, 100).unwrap_err().code,
            "request_replayed"
        );
        guard.consume("two", 130, 121).unwrap();
        let consumed = guard.consumed.lock().unwrap();
        assert!(!consumed.contains_key("one"));
        assert!(consumed.contains_key("two"));
    }

    #[test]
    fn success_receipt_requires_unchanged_workspace_generation_relay_and_signer() {
        let relay = "wss://buildcontext.communities.buzz.xyz";
        let pubkey = "a".repeat(64);
        assert!(ensure_scope_values_unchanged(7, relay, &pubkey, 7, relay, &pubkey).is_ok());
        for result in [
            ensure_scope_values_unchanged(7, relay, &pubkey, 8, relay, &pubkey),
            ensure_scope_values_unchanged(7, relay, &pubkey, 7, "wss://other", &pubkey),
            ensure_scope_values_unchanged(7, relay, &pubkey, 7, relay, &"b".repeat(64)),
        ] {
            assert_eq!(result.unwrap_err().code, "active_scope_changed");
        }
    }

    #[test]
    fn relay_is_assertion_only_and_canonical() {
        assert!(validate_relay("wss://buildcontext.communities.buzz.xyz").is_ok());
        assert!(validate_relay("https://buildcontext.communities.buzz.xyz/").is_ok());
        for rejected in [
            "wss://example.com",
            "http://buildcontext.communities.buzz.xyz",
            "wss://buildcontext.communities.buzz.xyz:8443",
            "wss://buildcontext.communities.buzz.xyz/query",
        ] {
            assert_eq!(validate_relay(rejected).unwrap_err().code, "relay_rejected");
        }
    }

    #[test]
    fn request_bounds_identity_range_and_search() {
        let now = 1_788_000_000;
        let mut candidate = request(now);
        candidate.limit = MAX_RESULTS + 1;
        assert_eq!(
            validate_request_at(&candidate, now).unwrap_err().code,
            "limit_rejected"
        );
        candidate.limit = 1;
        candidate.search = Some("x".repeat(MAX_SEARCH_CHARS + 1));
        assert_eq!(
            validate_request_at(&candidate, now).unwrap_err().code,
            "search_rejected"
        );
        candidate.search = None;
        candidate.expected_identity_pubkey = Some("not-a-pubkey".to_string());
        assert_eq!(
            validate_request_at(&candidate, now).unwrap_err().code,
            "identity_rejected"
        );
    }

    #[test]
    fn projection_bounds_redacts_and_filters_content() {
        let channel = "123e4567-e89b-12d3-a456-426614174000";
        let content = format!(
            "completed alpha BUZZ_PRIVATE_KEY={} Authorization=Bearer-abc secret=shh {}",
            "nsec1".to_string() + &"q".repeat(80),
            "x".repeat(800)
        );
        let event = EventBuilder::new(Kind::Custom(40002), content)
            .tags([Tag::parse(["h", channel]).unwrap()])
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let projected = project_event(&event, 160, &HashMap::new());
        let excerpt = projected.excerpt.unwrap();
        assert!(excerpt.chars().count() <= 160);
        assert!(!excerpt.contains("nsec1"));
        assert!(!excerpt.contains("Bearer-abc"));
        assert!(!excerpt.contains("secret=shh"));
        assert_eq!(bounded_excerpt("secret=shh", 4), "sec…");
        assert_eq!(bounded_excerpt("secret=shh", 1), "…");
        assert_eq!(bounded_excerpt("secret=shh", 0), "");

        let mut matching = request(1_788_000_000);
        matching.since = event.created_at.as_secs() as i64 - 1;
        matching.until = event.created_at.as_secs() as i64 + 1;
        matching.channel = Some(channel.to_string());
        matching.search = Some("ALPHA".to_string());
        assert!(event_matches_request(&event, &matching));
        matching.search = Some("missing".to_string());
        assert!(!event_matches_request(&event, &matching));

        let invalid_channel = EventBuilder::new(Kind::Custom(40002), "safe")
            .tags([Tag::parse(["h", "not-a-uuid\nsecret=bad"]).unwrap()])
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert!(project_event(&invalid_channel, 10, &HashMap::new())
            .channel
            .is_none());
    }

    #[test]
    fn event_signatures_are_verified() {
        let event = EventBuilder::text_note("verified")
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert!(verify_event_set(std::slice::from_ref(&event)).is_ok());
        let mut value = serde_json::to_value(event).unwrap();
        value["content"] = serde_json::json!("tampered");
        let tampered: Event = serde_json::from_value(value).unwrap();
        assert_eq!(
            verify_event_set(&[tampered]).unwrap_err().code,
            "response_unverified"
        );
    }

    #[test]
    fn owner_only_directory_and_socket_replacement_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("operator-read");
        ensure_owner_only_dir(&directory).unwrap();
        assert_eq!(
            fs::symlink_metadata(&directory)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            ensure_owner_only_dir(&directory).unwrap_err().code,
            "control_dir_rejected"
        );

        let socket_path = temp.path().join("desktop.sock");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&socket_path)
            .unwrap();
        file.write_all(b"replacement").unwrap();
        assert_eq!(
            prepare_socket_path(&socket_path).unwrap_err().code,
            "socket_rejected"
        );
    }

    #[test]
    fn bundled_client_symlink_is_created_refreshed_and_never_clobbers_regular_files() {
        let temp = tempfile::tempdir().unwrap();
        let app_bin = temp.path().join("Buzz.app/Contents/MacOS");
        let local_bin = temp.path().join("local-bin");
        fs::create_dir_all(&app_bin).unwrap();
        fs::write(app_bin.join("buzz-read"), b"credentialless client").unwrap();

        ensure_client_symlink_at(&app_bin, &local_bin).unwrap();
        let link = local_bin.join("buzz-read");
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(fs::read_link(&link).unwrap(), app_bin.join("buzz-read"));

        fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(temp.path().join("wrong"), &link).unwrap();
        ensure_client_symlink_at(&app_bin, &local_bin).unwrap();
        assert_eq!(fs::read_link(&link).unwrap(), app_bin.join("buzz-read"));

        fs::remove_file(&link).unwrap();
        fs::write(&link, b"user-owned client").unwrap();
        ensure_client_symlink_at(&app_bin, &local_bin).unwrap();
        assert_eq!(fs::read(&link).unwrap(), b"user-owned client");
    }

    #[tokio::test]
    async fn framed_socket_io_is_bounded() {
        let (mut left, mut right) = UnixStream::pair().unwrap();
        let writer = tokio::spawn(async move {
            write_frame_async(&mut left, b"hello", 5).await.unwrap();
        });
        assert_eq!(read_frame_async(&mut right, 5).await.unwrap(), b"hello");
        writer.await.unwrap();

        let (mut left, mut right) = UnixStream::pair().unwrap();
        let writer = tokio::spawn(async move {
            left.write_all(&6_u32.to_be_bytes()).await.unwrap();
        });
        assert_eq!(
            read_frame_async(&mut right, 5).await.unwrap_err().code,
            "request_oversize"
        );
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn unix_socket_peer_is_the_current_owner() {
        let (left, _right) = UnixStream::pair().unwrap();
        validate_peer(&left).unwrap();
        assert_eq!(
            ensure_peer_uid(current_uid().saturating_add(1), current_uid())
                .unwrap_err()
                .code,
            "peer_rejected"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn whole_request_timeout_cancels_in_flight_execution_at_expiry() {
        use std::sync::atomic::{AtomicBool, Ordering};

        struct DropSignal(Arc<AtomicBool>);
        impl Drop for DropSignal {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = DropSignal(cancelled.clone());
        let task = tokio::spawn(run_with_expiry_timeout(
            Duration::from_secs(5),
            async move {
                let _signal = signal;
                std::future::pending::<Result<(), OperatorError>>().await
            },
        ));
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(5)).await;

        assert_eq!(task.await.unwrap().unwrap_err().code, "request_stale");
        assert!(cancelled.load(Ordering::SeqCst));
    }

    #[test]
    fn receipt_bound_is_enforced() {
        let mut receipt = error_receipt("id".to_string(), &OperatorError::new("x", "y"));
        receipt.events = (0..MAX_RESULTS)
            .map(|index| ReceiptEvent {
                id: format!("{index:064x}"),
                author_pubkey: "a".repeat(64),
                author_name: Some("name".to_string()),
                kind: 40002,
                created_at: index as i64,
                channel: Some("123e4567-e89b-12d3-a456-426614174000".to_string()),
                excerpt: Some("x".repeat(MAX_EXCERPT_CHARS as usize)),
            })
            .collect();
        assert!(ensure_receipt_bound(&receipt).is_ok());
        receipt.events.push(ReceiptEvent {
            id: "z".repeat(MAX_RECEIPT_BYTES),
            author_pubkey: String::new(),
            author_name: None,
            kind: 9,
            created_at: 0,
            channel: None,
            excerpt: None,
        });
        assert_eq!(
            ensure_receipt_bound(&receipt).unwrap_err().code,
            "receipt_oversize"
        );
    }
}
