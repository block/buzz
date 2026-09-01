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
    identity_generation: u64,
    workspace_generation: u64,
}

#[derive(Debug)]
struct ProductionIdentity {
    keys: Keys,
    generation: u64,
}

#[derive(Clone, Copy)]
struct ScopeFingerprint<'a> {
    workspace_generation: u64,
    identity_generation: u64,
    relay: &'a str,
    pubkey: &'a str,
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
    let socket_metadata = validate_socket(&socket_path)?;
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
    let desktop_pid = validate_connected_server(&stream, &socket_path, &socket_metadata)?;

    let bytes = serde_json::to_vec(&request).map_err(|_| {
        OperatorError::new(
            "request_invalid",
            "the read request could not be serialized",
        )
    })?;
    write_frame_sync(&mut stream, &bytes, MAX_REQUEST_BYTES)?;
    let response = read_frame_sync(&mut stream, MAX_RECEIPT_BYTES)?;
    if validate_connected_server(&stream, &socket_path, &socket_metadata)? != desktop_pid {
        return Err(OperatorError::new(
            "server_rejected",
            "the connected Buzz Desktop process changed during the read",
        ));
    }
    let receipt: ReadReceipt = serde_json::from_slice(&response).map_err(|_| {
        OperatorError::new("receipt_invalid", "the Buzz read receipt was malformed")
    })?;
    validate_receipt_binding(&receipt, &request.request_id, desktop_pid)?;
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
    let valid_lifetime = request
        .expires_at
        .checked_sub(request.issued_at)
        .is_some_and(|lifetime| (1..=MAX_REQUEST_LIFETIME_SECONDS).contains(&lifetime));
    if request.issued_at > now.saturating_add(MAX_CLOCK_SKEW_SECONDS)
        || request.expires_at <= now
        || !valid_lifetime
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

include!("operator_read/server.rs");

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

include!("operator_read/tests.rs");
