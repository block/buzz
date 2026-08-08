use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use buzz_core::delivery_broker::{
    broker_response_digest, evaluate_top_level_mention_policy, is_brokered_message_kind,
    is_mention_policy_message_kind, validate_mention_policy_reply_parent,
    validate_top_level_mention_policy_config, BrokerErrorCode, BrokerOperation, BrokerRequest,
    BrokerResponse, BrokerResponseEnvelope, MentionPolicyDecision, MentionPolicyReplyContext,
    BROKER_CAPABILITY_ENV, BROKER_DIR_ENV, BROKER_PROTOCOL_VERSION,
    BROKER_RESPONSE_ATTESTATION_KIND, BROKER_RESPONSE_PUBKEY_ENV, MAX_BROKER_REQUEST_BYTES,
    MAX_BROKER_RESPONSE_BYTES, MAX_BROKER_RESULT_BYTES, TOP_LEVEL_MENTION_PUBKEYS_ENV,
};
use nostr::{Event, EventBuilder, Keys, Kind};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::relay::RestClient;

const REQUEST_MAX_AGE: Duration = Duration::from_secs(30);
const REQUEST_MAX_FUTURE_SKEW: Duration = Duration::from_secs(5);
const BROKER_POLL_INTERVAL: Duration = Duration::from_millis(20);
const MAX_FILTERS: usize = 8;
const MAX_FILTER_LIMIT: u64 = 5_000;
const DEFAULT_QUERY_LIMIT: u64 = 500;
const MAX_THREAD_DEPTH: u64 = 100;
const MAX_FEED_TYPES: usize = 16;
const MAX_MESSAGE_CONTENT_BYTES: usize = 64 * 1024;
const MAX_EVENT_TAGS: usize = 256;
const MAX_QUEUE_REJECTIONS_PER_POLL: usize = 8;
const MAX_SCAN_ENTRIES: usize = 256;
// Atomic staging files do not consume the valid-candidate budget, but retain a
// separate traversal ceiling so a cluttered writable request directory cannot
// monopolize the broker loop indefinitely. Normal clients create at most one
// short-lived staging file per in-flight request, so this leaves wide margin.
const MAX_SCAN_DIRECTORY_ENTRIES: usize = MAX_SCAN_ENTRIES * 4;
const STALE_RESPONSE_AGE: Duration = Duration::from_secs(120);
const MAX_CONCURRENT_REQUESTS: usize = 8;
const REQUEST_PROCESSING_TIMEOUT: Duration = Duration::from_secs(110);
const BROKER_ROOT_PREFIX: &str = "buzz-delivery-broker-";
const LEASE_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const LEASE_STALE_AGE: Duration = Duration::from_secs(10 * 60);
const LEGACY_ROOT_STALE_AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(serde::Serialize, serde::Deserialize)]
struct BrokerLease {
    pid: u32,
    instance_id: Uuid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProcessLiveness {
    Alive,
    Dead,
    Unknown,
}

pub(crate) struct DeliveryBroker {
    root: tempfile::TempDir,
    task: tokio::task::JoinHandle<()>,
    capability: String,
    response_pubkey: String,
    liveness_guard: Option<std::fs::File>,
}

impl DeliveryBroker {
    pub(crate) fn start(
        relay_url: &str,
        keys: Keys,
        auth_tag_json: Option<String>,
        top_level_mention_policy: Option<String>,
    ) -> anyhow::Result<Self> {
        if let Some(configured) = top_level_mention_policy.as_deref() {
            validate_top_level_mention_policy_config(configured)
                .map_err(|message| anyhow::anyhow!("{TOP_LEVEL_MENTION_PUBKEYS_ENV}: {message}"))?;
        }
        // Keep the broker parent outside the agent workspace. The Codex policy
        // receives only `<root>/requests` as an additional writable root, so it
        // cannot rename `processing`, `responses`, or the broker root itself.
        let broker_parent = dirs::data_local_dir()
            .ok_or_else(|| anyhow::anyhow!("local application data directory is unavailable"))?
            .join("buzz-acp")
            .join("delivery-brokers");
        std::fs::create_dir_all(&broker_parent).with_context(|| {
            format!("create delivery broker parent {}", broker_parent.display())
        })?;
        let broker_parent = std::fs::canonicalize(&broker_parent).with_context(|| {
            format!(
                "canonicalize delivery broker parent {}",
                broker_parent.display()
            )
        })?;
        let cwd = std::fs::canonicalize(std::env::current_dir()?)
            .context("canonicalize delivery broker working directory")?;
        if broker_parent.starts_with(&cwd) || cwd.starts_with(&broker_parent) {
            anyhow::bail!(
                "delivery broker parent {} overlaps agent workspace {}",
                broker_parent.display(),
                cwd.display()
            );
        }
        cleanup_stale_broker_roots(&broker_parent);
        let root = tempfile::Builder::new()
            .prefix(BROKER_ROOT_PREFIX)
            .tempdir_in(&broker_parent)
            .with_context(|| format!("create delivery broker under {}", broker_parent.display()))?;
        for child in ["requests", "processing", "responses"] {
            std::fs::create_dir(root.path().join(child))
                .with_context(|| format!("create delivery broker {child} directory"))?;
        }
        let instance_id = Uuid::new_v4();
        create_lease(root.path(), instance_id).with_context(|| {
            format!("create delivery broker lease in {}", root.path().display())
        })?;
        refresh_heartbeat(root.path()).with_context(|| {
            format!(
                "create delivery broker heartbeat in {}",
                root.path().display()
            )
        })?;
        let liveness_guard = create_liveness_guard(root.path()).with_context(|| {
            format!(
                "create delivery broker liveness guard in {}",
                root.path().display()
            )
        })?;

        let capability = random_capability();
        let response_keys = Keys::generate();
        let response_pubkey = response_keys.public_key().to_hex();
        let rest = RestClient::new(relay_url, keys, auth_tag_json)
            .map_err(|e| anyhow::anyhow!("create delivery broker relay client: {e}"))?;
        let task_root = root.path().to_path_buf();
        let task_capability = capability.clone();
        let task = tokio::spawn(async move {
            run_broker(
                task_root,
                task_capability,
                response_keys,
                rest,
                top_level_mention_policy,
            )
            .await;
        });

        Ok(Self {
            root,
            task,
            capability,
            response_pubkey,
            liveness_guard,
        })
    }

    pub(crate) fn environment(&self) -> anyhow::Result<[(String, String); 3]> {
        let root = self
            .root
            .path()
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("delivery broker path is not valid UTF-8"))?;
        Ok([
            (BROKER_DIR_ENV.into(), root.into()),
            (BROKER_CAPABILITY_ENV.into(), self.capability.clone()),
            (
                BROKER_RESPONSE_PUBKEY_ENV.into(),
                self.response_pubkey.clone(),
            ),
        ])
    }
}

impl Drop for DeliveryBroker {
    fn drop(&mut self) {
        self.task.abort();
        // On Windows the guard denies sharing so another process can prove the
        // broker is alive. Release it before TempDir attempts recursive cleanup.
        self.liveness_guard.take();
    }
}

async fn run_broker(
    root: PathBuf,
    capability: String,
    response_keys: Keys,
    rest: RestClient,
    top_level_mention_policy: Option<String>,
) {
    let mut last_cleanup = tokio::time::Instant::now();
    let mut last_lease_refresh = tokio::time::Instant::now();
    let mut jobs = tokio::task::JoinSet::new();
    loop {
        if last_lease_refresh.elapsed() >= LEASE_REFRESH_INTERVAL {
            if let Err(error) = refresh_heartbeat(&root) {
                tracing::warn!("delivery broker heartbeat refresh failed: {error}");
            }
            last_lease_refresh = tokio::time::Instant::now();
        }
        if last_cleanup.elapsed() >= Duration::from_secs(5) {
            prune_stale_files(&root.join("requests"), STALE_RESPONSE_AGE, MAX_SCAN_ENTRIES);
            prune_stale_files(
                &root.join("processing"),
                STALE_RESPONSE_AGE,
                MAX_SCAN_ENTRIES,
            );
            prune_stale_files(
                &root.join("responses"),
                STALE_RESPONSE_AGE,
                MAX_SCAN_ENTRIES,
            );
            last_cleanup = tokio::time::Instant::now();
        }
        while jobs.len() < MAX_CONCURRENT_REQUESTS {
            match next_request_path(&root) {
                Ok(Some((request_id, path))) => {
                    let job_root = root.clone();
                    let job_capability = capability.clone();
                    let job_response_keys = response_keys.clone();
                    let job_rest = rest.clone();
                    let job_mention_policy = top_level_mention_policy.clone();
                    jobs.spawn(async move {
                        handle_claimed_request(
                            &job_root,
                            request_id,
                            path,
                            &job_capability,
                            &job_response_keys,
                            &job_rest,
                            job_mention_policy.as_deref(),
                        )
                        .await;
                    });
                }
                Ok(None) => break,
                Err(error) => {
                    tracing::warn!("delivery broker request scan failed: {error}");
                    break;
                }
            }
        }
        if jobs.len() >= MAX_CONCURRENT_REQUESTS {
            reject_queued_requests(&root, &response_keys);
        }

        tokio::select! {
            _ = tokio::time::sleep(BROKER_POLL_INTERVAL) => {}
            completed = jobs.join_next(), if !jobs.is_empty() => {
                if let Some(Err(error)) = completed {
                    tracing::warn!("delivery broker request task failed: {error}");
                }
            }
        }
    }
}

fn create_lease(root: &Path, instance_id: Uuid) -> std::io::Result<()> {
    let lease = root.join("lease");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = options.open(lease)?;
    if metadata_is_reparse_point(&file.metadata()?) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "delivery broker lease is a reparse point",
        ));
    }
    let lease = BrokerLease {
        pid: std::process::id(),
        instance_id,
    };
    serde_json::to_writer(&mut file, &lease).map_err(std::io::Error::other)?;
    file.sync_all()
}

fn refresh_heartbeat(root: &Path) -> std::io::Result<()> {
    let heartbeat = root.join("heartbeat");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = options.open(heartbeat)?;
    if metadata_is_reparse_point(&file.metadata()?) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "delivery broker heartbeat is a reparse point",
        ));
    }
    file.write_all(b"1")?;
    file.sync_all()
}

fn create_liveness_guard(root: &Path) -> std::io::Result<Option<std::fs::File>> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let guard = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(root.join("liveness.lock"))?;
        return Ok(Some(guard));
    }
    #[cfg(not(windows))]
    {
        let _ = root;
        Ok(None)
    }
}

fn cleanup_stale_broker_roots(parent: &Path) {
    let now = SystemTime::now();
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.take(MAX_SCAN_ENTRIES).flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with(BROKER_ROOT_PREFIX) {
            continue;
        }
        let Ok(root_metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if root_metadata.file_type().is_symlink()
            || metadata_is_reparse_point(&root_metadata)
            || !root_metadata.is_dir()
        {
            continue;
        }
        let root_modified = root_metadata.modified().ok();
        let lease_path = path.join("lease");
        let reap = match std::fs::symlink_metadata(&lease_path) {
            Ok(metadata)
                if metadata.is_file()
                    && !metadata.file_type().is_symlink()
                    && !metadata_is_reparse_point(&metadata) =>
            {
                let freshness_metadata = std::fs::symlink_metadata(path.join("heartbeat"))
                    .ok()
                    .filter(|heartbeat| {
                        heartbeat.is_file()
                            && !heartbeat.file_type().is_symlink()
                            && !metadata_is_reparse_point(heartbeat)
                    });
                let lease_stale = freshness_metadata
                    .as_ref()
                    .unwrap_or(&metadata)
                    .modified()
                    .ok()
                    .and_then(|modified| now.duration_since(modified).ok())
                    .is_some_and(|age| age >= LEASE_STALE_AGE);
                let parsed_lease = read_bounded_regular_file(&lease_path, 4 * 1024)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<BrokerLease>(&bytes).ok());
                match parsed_lease {
                    Some(lease) => lease_root_is_reapable(
                        lease_stale,
                        lease_process_liveness(lease.pid, &path.join("liveness.lock")),
                    ),
                    None => legacy_root_is_stale(now, root_modified),
                }
            }
            Ok(_) => false,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                legacy_root_is_stale(now, root_modified)
            }
            Err(_) => false,
        };
        if reap {
            if let Err(error) = std::fs::remove_dir_all(&path) {
                tracing::warn!(path = %path.display(), "stale delivery broker cleanup failed: {error}");
            }
        }
    }
}

fn legacy_root_is_stale(now: SystemTime, root_modified: Option<SystemTime>) -> bool {
    root_modified
        .and_then(|modified| now.duration_since(modified).ok())
        .is_some_and(|age| age >= LEGACY_ROOT_STALE_AGE)
}

fn lease_root_is_reapable(stale: bool, liveness: ProcessLiveness) -> bool {
    stale && liveness == ProcessLiveness::Dead
}

#[cfg(unix)]
fn process_liveness(pid: u32) -> ProcessLiveness {
    let Ok(pid) = i32::try_from(pid) else {
        return ProcessLiveness::Unknown;
    };
    match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None) {
        Ok(()) => ProcessLiveness::Alive,
        Err(nix::errno::Errno::ESRCH) => ProcessLiveness::Dead,
        Err(_) => ProcessLiveness::Unknown,
    }
}

#[cfg(windows)]
fn lease_process_liveness(_pid: u32, lock_path: &Path) -> ProcessLiveness {
    use std::os::windows::fs::OpenOptionsExt;
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(lock_path)
    {
        Ok(_) => ProcessLiveness::Dead,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            ProcessLiveness::Alive
        }
        Err(_) => ProcessLiveness::Unknown,
    }
}

#[cfg(unix)]
fn lease_process_liveness(pid: u32, _lock_path: &Path) -> ProcessLiveness {
    process_liveness(pid)
}

#[cfg(not(any(unix, windows)))]
fn lease_process_liveness(_pid: u32, _lock_path: &Path) -> ProcessLiveness {
    ProcessLiveness::Unknown
}

async fn handle_claimed_request(
    root: &Path,
    request_id: Uuid,
    path: PathBuf,
    capability: &str,
    response_keys: &Keys,
    rest: &RestClient,
    top_level_mention_policy: Option<&str>,
) {
    let response = match tokio::time::timeout(
        REQUEST_PROCESSING_TIMEOUT,
        process_request_file(
            request_id,
            &path,
            capability,
            rest,
            top_level_mention_policy,
        ),
    )
    .await
    {
        Ok(response) => response,
        Err(_) => BrokerResponse::failure(
            request_id,
            BrokerErrorCode::DeliveryUnknown,
            "delivery broker request processing timed out",
        ),
    };
    write_claimed_response(root, request_id, path, response, response_keys);
}

fn write_claimed_response(
    root: &Path,
    request_id: Uuid,
    path: PathBuf,
    response: BrokerResponse,
    response_keys: &Keys,
) {
    let response_path = root.join("responses").join(format!("{request_id}.json"));
    match encode_signed_response(response, response_keys) {
        Ok(bytes) => {
            if let Err(error) = write_atomic(&response_path, &bytes) {
                tracing::warn!(
                    request_id = %request_id,
                    "delivery broker response write failed: {error}"
                );
            }
        }
        Err(error) => tracing::warn!(
            request_id = %request_id,
            "delivery broker response encode failed: {error}"
        ),
    }
    let _ = std::fs::remove_file(path);
}

fn reject_queued_requests(root: &Path, response_keys: &Keys) {
    for _ in 0..MAX_QUEUE_REJECTIONS_PER_POLL {
        match next_request_path(root) {
            Ok(Some((request_id, path))) => write_claimed_response(
                root,
                request_id,
                path,
                BrokerResponse::failure(
                    request_id,
                    BrokerErrorCode::Busy,
                    "delivery broker is at its in-flight limit; request was not executed",
                ),
                response_keys,
            ),
            Ok(None) => break,
            Err(error) => {
                tracing::warn!("delivery broker overflow scan failed: {error}");
                break;
            }
        }
    }
}

fn encode_signed_response(
    response: BrokerResponse,
    response_keys: &Keys,
) -> Result<Vec<u8>, String> {
    let request_id = response.request_id;
    let envelope = signed_response_envelope(response, response_keys)?;
    let bytes = serde_json::to_vec(&envelope).map_err(|error| error.to_string())?;
    if bytes.len() as u64 <= MAX_BROKER_RESPONSE_BYTES {
        return Ok(bytes);
    }

    let failure = BrokerResponse::failure(
        request_id,
        BrokerErrorCode::Internal,
        "delivery broker response envelope exceeded its size limit",
    );
    let envelope = signed_response_envelope(failure, response_keys)?;
    let bytes = serde_json::to_vec(&envelope).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BROKER_RESPONSE_BYTES {
        return Err("delivery broker error envelope exceeded its size limit".into());
    }
    Ok(bytes)
}

fn signed_response_envelope(
    response: BrokerResponse,
    response_keys: &Keys,
) -> Result<BrokerResponseEnvelope, String> {
    let content = broker_response_digest(&response).map_err(|error| error.to_string())?;
    let attestation = EventBuilder::new(Kind::Custom(BROKER_RESPONSE_ATTESTATION_KIND), content)
        .tags([])
        .sign_with_keys(response_keys)
        .map_err(|error| error.to_string())?;
    Ok(BrokerResponseEnvelope {
        response,
        attestation,
    })
}

fn next_request_path(root: &Path) -> std::io::Result<Option<(Uuid, PathBuf)>> {
    let requests = root.join("requests");
    let processing = root.join("processing");
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(requests)?.take(MAX_SCAN_DIRECTORY_ENTRIES) {
        let entry = entry?;
        let source = entry.path();
        if source
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.starts_with('.'))
        {
            // The CLI creates dot-prefixed temporary files in this directory
            // and atomically renames them to `<request-id>.json` only after a
            // complete fsync. Never unlink an in-progress atomic write.
            continue;
        }
        let metadata = std::fs::symlink_metadata(&source)?;
        if metadata.file_type().is_symlink() || metadata_is_reparse_point(&metadata) {
            let _ = std::fs::remove_file(&source);
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        let Some(stem) = source.file_stem().and_then(|value| value.to_str()) else {
            let _ = std::fs::remove_file(&source);
            continue;
        };
        if source.extension().and_then(|value| value.to_str()) != Some("json") {
            let _ = std::fs::remove_file(&source);
            continue;
        }
        let Ok(request_id) = Uuid::parse_str(stem) else {
            let _ = std::fs::remove_file(&source);
            continue;
        };
        candidates.push((request_id, source));
        if candidates.len() >= MAX_SCAN_ENTRIES {
            break;
        }
    }

    candidates.sort_by(|left, right| left.1.cmp(&right.1));
    for (request_id, source) in candidates {
        let claimed = processing.join(format!("{request_id}.json"));
        match std::fs::rename(&source, &claimed) {
            Ok(()) => return Ok(Some((request_id, claimed))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

fn prune_stale_files(directory: &Path, max_age: Duration, max_entries: usize) {
    let now = SystemTime::now();
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.take(max_entries).flatten() {
        let path = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() || metadata_is_reparse_point(&metadata) {
            let _ = std::fs::remove_file(path);
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        let stale = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= max_age);
        if stale {
            let _ = std::fs::remove_file(path);
        }
    }
}

async fn process_request_file(
    filename_request_id: Uuid,
    path: &Path,
    expected_capability: &str,
    rest: &RestClient,
    top_level_mention_policy: Option<&str>,
) -> BrokerResponse {
    let bytes = match read_bounded_regular_file(path, MAX_BROKER_REQUEST_BYTES) {
        Ok(bytes) => bytes,
        Err(error) => {
            return BrokerResponse::failure(
                filename_request_id,
                BrokerErrorCode::InvalidRequest,
                format!("request read failed: {error}"),
            )
        }
    };
    let request: BrokerRequest = match serde_json::from_slice(&bytes) {
        Ok(request) => request,
        Err(error) => {
            return BrokerResponse::failure(
                filename_request_id,
                BrokerErrorCode::InvalidRequest,
                format!("request decode failed: {error}"),
            )
        }
    };

    if request.version != BROKER_PROTOCOL_VERSION || request.request_id != filename_request_id {
        return BrokerResponse::failure(
            filename_request_id,
            BrokerErrorCode::InvalidRequest,
            "request protocol version or identifier mismatch",
        );
    }
    if !capabilities_equal(&request.capability, expected_capability) {
        return BrokerResponse::failure(
            filename_request_id,
            BrokerErrorCode::Unauthorized,
            "invalid delivery broker capability",
        );
    }
    if !request_time_is_valid(request.created_at_ms) {
        return BrokerResponse::failure(
            filename_request_id,
            BrokerErrorCode::InvalidRequest,
            "delivery broker request is stale or from the future",
        );
    }

    match request.operation {
        BrokerOperation::Query { mut filters } => {
            if let Err(error) = validate_filters(&mut filters, true) {
                return BrokerResponse::failure(
                    filename_request_id,
                    BrokerErrorCode::InvalidRequest,
                    error,
                );
            }
            match rest.query_values(&filters).await {
                Ok(value) => bounded_result_response(filename_request_id, value),
                Err(error) => BrokerResponse::failure(
                    filename_request_id,
                    BrokerErrorCode::Internal,
                    sanitize_detail(&error.to_string()),
                ),
            }
        }
        BrokerOperation::Count { mut filters } => {
            if let Err(error) = validate_filters(&mut filters, false) {
                return BrokerResponse::failure(
                    filename_request_id,
                    BrokerErrorCode::InvalidRequest,
                    error,
                );
            }
            match rest.count_values(&filters).await {
                Ok(value) => bounded_result_response(filename_request_id, value),
                Err(error) => BrokerResponse::failure(
                    filename_request_id,
                    BrokerErrorCode::Internal,
                    sanitize_detail(&error.to_string()),
                ),
            }
        }
        BrokerOperation::SubmitStoredMessage { event } => {
            match submit_verified_message_with_policy(rest, &event, top_level_mention_policy).await
            {
                Ok(value) => BrokerResponse::success(filename_request_id, value),
                Err((code, message)) => BrokerResponse::failure(filename_request_id, code, message),
            }
        }
    }
}

fn bounded_result_response(request_id: Uuid, value: serde_json::Value) -> BrokerResponse {
    match serde_json::to_vec(&value) {
        Ok(bytes) if bytes.len() as u64 <= MAX_BROKER_RESULT_BYTES => {
            BrokerResponse::success(request_id, value)
        }
        Ok(_) => BrokerResponse::failure(
            request_id,
            BrokerErrorCode::Internal,
            "delivery broker relay result exceeded its size limit",
        ),
        Err(error) => BrokerResponse::failure(
            request_id,
            BrokerErrorCode::Internal,
            format!("delivery broker response serialization failed: {error}"),
        ),
    }
}

#[cfg(test)]
async fn submit_verified_message(
    rest: &RestClient,
    event: &Event,
) -> Result<serde_json::Value, (BrokerErrorCode, String)> {
    submit_verified_message_with_policy(rest, event, None).await
}

async fn submit_verified_message_with_policy(
    rest: &RestClient,
    event: &Event,
    top_level_mention_policy: Option<&str>,
) -> Result<serde_json::Value, (BrokerErrorCode, String)> {
    let kind = event.kind.as_u16();
    if !is_brokered_message_kind(kind) {
        return Err((
            BrokerErrorCode::Unsupported,
            format!("event kind {kind} is not allowed by the delivery broker"),
        ));
    }
    if event.pubkey != rest.keys.public_key() {
        return Err((
            BrokerErrorCode::Unauthorized,
            "event signer does not match the harness identity".into(),
        ));
    }
    if event.content.len() > MAX_MESSAGE_CONTENT_BYTES || event.tags.len() > MAX_EVENT_TAGS {
        return Err((
            BrokerErrorCode::InvalidRequest,
            "event content or tag count exceeds the broker limit".into(),
        ));
    }
    event.verify().map_err(|error| {
        (
            BrokerErrorCode::InvalidRequest,
            format!("event signature verification failed: {error}"),
        )
    })?;
    if is_mention_policy_message_kind(kind) {
        if let Some(configured) = top_level_mention_policy {
            match evaluate_top_level_mention_policy(event, configured)
                .map_err(|message| (BrokerErrorCode::InvalidRequest, message))?
            {
                MentionPolicyDecision::Allow => {}
                MentionPolicyDecision::VerifyReply(context) => {
                    verify_mention_policy_reply_parent(rest, &context).await?;
                }
            }
        }
    }

    let expected_id = event.id.to_hex();
    match exact_readback_once(rest, event).await {
        Ok(true) => {
            return Ok(serde_json::json!({
                "event_id": expected_id,
                "accepted": true,
                "message": "event was already present on exact readback",
                "delivery_path": "harness_broker",
                "readback_verified": true,
                "reconciled": true,
            }));
        }
        Ok(false) | Err(ExactReadbackError::Transport(_)) => {}
        Err(ExactReadbackError::Invalid(message)) => {
            return Err((BrokerErrorCode::DeliveryUnknown, message));
        }
    }
    let submission = rest.submit_event(event).await;
    let readback = verify_exact_readback(rest, event).await;

    match (submission, readback) {
        (Ok(receipt), Ok(())) => {
            let mut object = receipt.as_object().cloned().unwrap_or_default();
            let accepted = object
                .get("accepted")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let receipt_id = object
                .get("event_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let reconciled = !accepted || receipt_id != expected_id;
            object.insert("event_id".into(), serde_json::json!(expected_id));
            object.insert("accepted".into(), serde_json::json!(true));
            object.insert("delivery_path".into(), serde_json::json!("harness_broker"));
            object.insert("readback_verified".into(), serde_json::json!(true));
            object.insert("reconciled".into(), serde_json::json!(reconciled));
            Ok(serde_json::Value::Object(object))
        }
        (Ok(receipt), Err(readback_error)) => {
            let accepted = receipt
                .get("accepted")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let receipt_id_matches = receipt
                .get("event_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|event_id| event_id == expected_id);
            let message = receipt
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("relay did not accept the event");
            if !accepted {
                Err((
                    BrokerErrorCode::RelayRejected,
                    sanitize_detail(&format!(
                        "{message}; exact readback failed: {readback_error}"
                    )),
                ))
            } else if !receipt_id_matches {
                Err((
                    BrokerErrorCode::DeliveryUnknown,
                    sanitize_detail(&format!(
                        "relay receipt event id did not match; exact readback failed: {readback_error}"
                    )),
                ))
            } else {
                Err((
                    BrokerErrorCode::DeliveryUnknown,
                    sanitize_detail(&format!(
                        "relay accepted the event but exact readback failed: {readback_error}"
                    )),
                ))
            }
        }
        (Err(_submit_error), Ok(())) => Ok(serde_json::json!({
            "event_id": expected_id,
            "accepted": true,
            "message": "submission outcome reconciled by exact event readback",
            "delivery_path": "harness_broker",
            "readback_verified": true,
            "reconciled": true,
        })),
        (Err(submit_error), Err(readback_error)) => Err((
            BrokerErrorCode::DeliveryUnknown,
            sanitize_detail(&format!(
                "event submission failed: {submit_error}; exact readback failed: {readback_error}"
            )),
        )),
    }
}

async fn verify_mention_policy_reply_parent(
    rest: &RestClient,
    context: &MentionPolicyReplyContext,
) -> Result<(), (BrokerErrorCode, String)> {
    let filters = [serde_json::json!({
        "ids": [context.parent_event_id.clone()],
        "limit": 1
    })];
    let value = rest.query_values(&filters).await.map_err(|error| {
        (
            BrokerErrorCode::Internal,
            sanitize_detail(&format!("reply parent query failed: {error}")),
        )
    })?;
    let parents: Vec<Event> = serde_json::from_value(value).map_err(|error| {
        (
            BrokerErrorCode::Internal,
            sanitize_detail(&format!("reply parent response was invalid: {error}")),
        )
    })?;
    let mut matching = parents
        .into_iter()
        .filter(|parent| parent.id.to_hex() == context.parent_event_id);
    let parent = matching.next().ok_or_else(|| {
        (
            BrokerErrorCode::InvalidRequest,
            format!("reply parent {} was not found", context.parent_event_id),
        )
    })?;
    if matching.next().is_some() {
        return Err((
            BrokerErrorCode::InvalidRequest,
            "relay returned duplicate reply parents".into(),
        ));
    }
    validate_mention_policy_reply_parent(&parent, context)
        .map_err(|message| (BrokerErrorCode::InvalidRequest, message))
}

pub(crate) fn normalize_top_level_mention_policy(
    persona_env_vars: &mut Vec<(String, String)>,
) -> anyhow::Result<Option<String>> {
    let parent = match std::env::var(TOP_LEVEL_MENTION_PUBKEYS_ENV) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            anyhow::bail!("{TOP_LEVEL_MENTION_PUBKEYS_ENV} is not valid UTF-8")
        }
    };
    normalize_top_level_mention_policy_with_parent(parent, persona_env_vars)
}

fn normalize_top_level_mention_policy_with_parent(
    parent: Option<String>,
    persona_env_vars: &mut Vec<(String, String)>,
) -> anyhow::Result<Option<String>> {
    let selected = select_top_level_mention_policy(parent, persona_env_vars);
    if let Some(configured) = selected.as_deref() {
        validate_top_level_mention_policy_config(configured)
            .map_err(|message| anyhow::anyhow!("{TOP_LEVEL_MENTION_PUBKEYS_ENV}: {message}"))?;
    }
    persona_env_vars.retain(|(key, _)| key != TOP_LEVEL_MENTION_PUBKEYS_ENV);
    if let Some(configured) = selected.as_ref() {
        persona_env_vars.push((TOP_LEVEL_MENTION_PUBKEYS_ENV.into(), configured.clone()));
    }
    Ok(selected)
}

fn select_top_level_mention_policy(
    parent: Option<String>,
    persona_env_vars: &[(String, String)],
) -> Option<String> {
    parent.or_else(|| {
        persona_env_vars
            .iter()
            .rev()
            .find(|(key, _)| key == TOP_LEVEL_MENTION_PUBKEYS_ENV)
            .map(|(_, value)| value.clone())
    })
}

async fn verify_exact_readback(rest: &RestClient, expected: &Event) -> Result<(), String> {
    let delays = [50_u64, 100, 200, 400];
    let mut last_error = None;
    for (index, delay_ms) in delays.iter().copied().enumerate() {
        match exact_readback_once(rest, expected).await {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(ExactReadbackError::Transport(error)) => last_error = Some(error),
            Err(ExactReadbackError::Invalid(error)) => return Err(error),
        }
        if index + 1 < delays.len() {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
    }
    Err(last_error.unwrap_or_else(|| "accepted event was not visible on exact readback".into()))
}

enum ExactReadbackError {
    Transport(String),
    Invalid(String),
}

async fn exact_readback_once(
    rest: &RestClient,
    expected: &Event,
) -> Result<bool, ExactReadbackError> {
    let filter = serde_json::json!({"ids": [expected.id.to_hex()], "limit": 1});
    let value = rest
        .query_values(std::slice::from_ref(&filter))
        .await
        .map_err(|error| ExactReadbackError::Transport(sanitize_detail(&error.to_string())))?;
    let events = value.as_array().ok_or_else(|| {
        ExactReadbackError::Invalid("relay readback response was not an event array".into())
    })?;
    let Some(found) = events.first() else {
        return Ok(false);
    };
    let event = serde_json::from_value::<Event>(found.clone())
        .map_err(|error| ExactReadbackError::Invalid(format!("invalid readback event: {error}")))?;
    if event.verify().is_ok() && event == *expected {
        Ok(true)
    } else {
        Err(ExactReadbackError::Invalid(
            "relay readback did not exactly match the signed event".into(),
        ))
    }
}

fn validate_filters(filters: &mut [serde_json::Value], is_query: bool) -> Result<(), String> {
    if filters.is_empty() || filters.len() > MAX_FILTERS {
        return Err(format!("filter count must be between 1 and {MAX_FILTERS}"));
    }
    for filter in filters {
        let object = filter
            .as_object_mut()
            .ok_or_else(|| "each filter must be a JSON object".to_string())?;
        for key in object.keys() {
            let allowed = matches!(
                key.as_str(),
                "ids"
                    | "authors"
                    | "kinds"
                    | "since"
                    | "until"
                    | "limit"
                    | "search"
                    | "before_id"
                    | "depth_limit"
                    | "feed_types"
            ) || key.starts_with('#');
            if !allowed {
                return Err(format!("unsupported filter field: {key}"));
            }
        }
        match object.get("limit").and_then(serde_json::Value::as_u64) {
            Some(limit) if limit <= MAX_FILTER_LIMIT => {}
            Some(_) => return Err(format!("filter limit exceeds {MAX_FILTER_LIMIT}")),
            None if is_query => {
                object.insert("limit".into(), serde_json::json!(DEFAULT_QUERY_LIMIT));
            }
            None => {}
        }
        if object
            .get("depth_limit")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|depth| depth > MAX_THREAD_DEPTH)
        {
            return Err(format!("depth_limit exceeds {MAX_THREAD_DEPTH}"));
        }
        if let Some(feed_types) = object.get("feed_types") {
            let values = feed_types
                .as_array()
                .ok_or_else(|| "feed_types must be an array".to_string())?;
            if values.len() > MAX_FEED_TYPES || values.iter().any(|value| !value.is_string()) {
                return Err(format!(
                    "feed_types must contain at most {MAX_FEED_TYPES} strings"
                ));
            }
        }
    }
    Ok(())
}

fn request_time_is_valid(created_at_ms: u64) -> bool {
    let now = unix_now_ms();
    created_at_ms <= now.saturating_add(REQUEST_MAX_FUTURE_SKEW.as_millis() as u64)
        && now.saturating_sub(created_at_ms) <= REQUEST_MAX_AGE.as_millis() as u64
}

fn capabilities_equal(provided: &str, expected: &str) -> bool {
    provided.len() == expected.len() && bool::from(provided.as_bytes().ct_eq(expected.as_bytes()))
}

fn random_capability() -> String {
    use rand::Rng;
    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn sanitize_detail(detail: &str) -> String {
    detail
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(512)
        .collect()
}

fn read_bounded_regular_file(path: &Path, max_bytes: u64) -> std::io::Result<Vec<u8>> {
    let mut file = open_read_nofollow(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata_is_reparse_point(&metadata) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "broker request is not a regular file",
        ));
    }
    if metadata.len() > max_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "broker request exceeds size limit",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    Read::by_ref(&mut file)
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "broker request exceeds size limit",
        ));
    }
    Ok(bytes)
}

fn open_read_nofollow(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        // Open the reparse point itself instead of traversing it. The opened
        // handle's metadata is then checked for FILE_ATTRIBUTE_REPARSE_POINT.
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options.open(path)
}

fn metadata_is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent")
    })?;
    let temp_path = parent.join(format!(".{}.tmp", Uuid::new_v4()));

    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp_path)?
    };
    #[cfg(not(unix))]
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)?;

    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp_path, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Bytes, extract::State, routing::post, Json, Router};
    use nostr::{EventBuilder, Kind};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };

    #[derive(Clone)]
    struct TestRelayState {
        stored: Arc<Mutex<Option<Event>>>,
        store_on_submit: bool,
        receipt: serde_json::Value,
        submit_count: Arc<AtomicUsize>,
    }

    async fn test_query(
        State(state): State<TestRelayState>,
        body: Bytes,
    ) -> Json<serde_json::Value> {
        let filters: serde_json::Value =
            serde_json::from_slice(&body).expect("query filter payload");
        let requested_ids = filters
            .as_array()
            .and_then(|filters| filters.first())
            .and_then(|filter| filter.get("ids"))
            .and_then(serde_json::Value::as_array)
            .map(|ids| {
                ids.iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
            });
        let events = state
            .stored
            .lock()
            .expect("stored lock")
            .clone()
            .into_iter()
            .filter(|event| {
                requested_ids
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&event.id.to_hex().as_str()))
            })
            .collect::<Vec<_>>();
        Json(serde_json::to_value(events).expect("events json"))
    }

    async fn test_submit(
        State(state): State<TestRelayState>,
        body: Bytes,
    ) -> Json<serde_json::Value> {
        state.submit_count.fetch_add(1, Ordering::SeqCst);
        let event: Event = serde_json::from_slice(&body).expect("submitted event");
        if state.store_on_submit {
            *state.stored.lock().expect("stored lock") = Some(event);
        }
        Json(state.receipt)
    }

    async fn spawn_test_relay(
        initial: Option<Event>,
        store_on_submit: bool,
        receipt: serde_json::Value,
    ) -> (RestClient, Arc<AtomicUsize>) {
        let submit_count = Arc::new(AtomicUsize::new(0));
        let state = TestRelayState {
            stored: Arc::new(Mutex::new(initial)),
            store_on_submit,
            receipt,
            submit_count: submit_count.clone(),
        };
        let app = Router::new()
            .route("/query", post(test_query))
            .route("/events", post(test_submit))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("test relay");
        });
        let keys = Keys::generate();
        let rest = RestClient::new(&format!("http://{address}"), keys, None).expect("rest client");
        (rest, submit_count)
    }

    #[test]
    fn filter_validation_is_bounded_and_allowlisted() {
        let mut bounded = vec![serde_json::json!({"kinds": [9], "limit": 1})];
        assert!(validate_filters(&mut bounded, true).is_ok());

        let mut normalized = vec![serde_json::json!({
            "kinds": [9],
            "depth_limit": 4,
            "feed_types": ["mentions", "needs_action"]
        })];
        assert!(validate_filters(&mut normalized, true).is_ok());
        assert_eq!(normalized[0]["limit"], DEFAULT_QUERY_LIMIT);

        let mut invalid = vec![serde_json::json!({
            "kinds": [9],
            "limit": 1,
            "path": "/admin"
        })];
        assert!(validate_filters(&mut invalid, true).is_err());
    }

    #[tokio::test]
    async fn submit_validation_rejects_wrong_kind_and_signer() {
        let harness_keys = Keys::generate();
        let other_keys = Keys::generate();
        let rest = RestClient {
            http: reqwest::Client::new(),
            base_url: "http://127.0.0.1".into(),
            keys: harness_keys.clone(),
            auth_tag_json: None,
        };
        let wrong_kind = EventBuilder::new(Kind::Custom(1), "content")
            .tags([])
            .sign_with_keys(&harness_keys)
            .expect("sign");
        let wrong_signer = EventBuilder::new(Kind::Custom(9), "content")
            .tags([])
            .sign_with_keys(&other_keys)
            .expect("sign");
        let mut bad_signature = EventBuilder::new(Kind::Custom(9), "content")
            .tags([])
            .sign_with_keys(&harness_keys)
            .expect("sign");
        bad_signature.content.push_str(" tampered");

        let kind_error = submit_verified_message(&rest, &wrong_kind)
            .await
            .expect_err("wrong kind");
        let signer_error = submit_verified_message(&rest, &wrong_signer)
            .await
            .expect_err("wrong signer");
        let signature_error = submit_verified_message(&rest, &bad_signature)
            .await
            .expect_err("bad signature");
        assert_eq!(kind_error.0, BrokerErrorCode::Unsupported);
        assert_eq!(signer_error.0, BrokerErrorCode::Unauthorized);
        assert_eq!(signature_error.0, BrokerErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn configured_mention_policy_rejects_before_broker_relay_submission() {
        let keys = Keys::generate();
        let required_recipient = Keys::generate().public_key();
        let wrong_recipient = Keys::generate().public_key();
        let event = EventBuilder::new(Kind::Custom(9), "wrong recipient")
            .tags([nostr::Tag::public_key(wrong_recipient)])
            .sign_with_keys(&keys)
            .expect("event");
        let (mut rest, submit_count) = spawn_test_relay(
            None,
            true,
            serde_json::json!({
                "accepted": true,
                "event_id": event.id.to_hex(),
                "message": "stored"
            }),
        )
        .await;
        rest.keys = keys;

        let error =
            submit_verified_message_with_policy(&rest, &event, Some(&required_recipient.to_hex()))
                .await
                .expect_err("policy mismatch must fail closed");

        assert_eq!(error.0, BrokerErrorCode::InvalidRequest);
        assert_eq!(submit_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn configured_mention_policy_verifies_unmentioned_reply_parent() {
        let keys = Keys::generate();
        let parent_keys = Keys::generate();
        let required_recipient = Keys::generate().public_key();
        let channel_id = Uuid::new_v4().to_string();
        let parent = EventBuilder::new(Kind::Custom(9), "parent")
            .tags([nostr::Tag::parse(["h", &channel_id]).expect("parent channel")])
            .sign_with_keys(&parent_keys)
            .expect("parent event");
        let reply = EventBuilder::new(Kind::Custom(9), "reply")
            .tags([
                nostr::Tag::parse(["h", &channel_id]).expect("reply channel"),
                nostr::Tag::parse(["e", &parent.id.to_hex(), "", "reply"]).expect("reply marker"),
            ])
            .sign_with_keys(&keys)
            .expect("reply event");
        let (mut rest, submit_count) = spawn_test_relay(
            Some(parent),
            true,
            serde_json::json!({
                "accepted": true,
                "event_id": reply.id.to_hex(),
                "message": "stored"
            }),
        )
        .await;
        rest.keys = keys;

        let result =
            submit_verified_message_with_policy(&rest, &reply, Some(&required_recipient.to_hex()))
                .await
                .expect("signed same-channel reply");

        assert_eq!(result["event_id"], reply.id.to_hex());
        assert_eq!(result["readback_verified"], true);
        assert_eq!(submit_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn configured_mention_policy_rejects_forged_or_cross_channel_reply() {
        let keys = Keys::generate();
        let required_recipient = Keys::generate().public_key();
        let parent_channel = Uuid::new_v4().to_string();
        let reply_channel = Uuid::new_v4().to_string();
        let parent = EventBuilder::new(Kind::Custom(9), "parent")
            .tags([nostr::Tag::parse(["h", &parent_channel]).expect("parent channel")])
            .sign_with_keys(&Keys::generate())
            .expect("parent event");
        let reply = EventBuilder::new(Kind::Custom(9), "reply")
            .tags([
                nostr::Tag::parse(["h", &reply_channel]).expect("reply channel"),
                nostr::Tag::parse(["e", &parent.id.to_hex(), "", "reply"]).expect("reply marker"),
            ])
            .sign_with_keys(&keys)
            .expect("reply event");
        let (mut rest, submit_count) = spawn_test_relay(
            Some(parent),
            true,
            serde_json::json!({"accepted": true, "event_id": reply.id.to_hex()}),
        )
        .await;
        rest.keys = keys;

        let error =
            submit_verified_message_with_policy(&rest, &reply, Some(&required_recipient.to_hex()))
                .await
                .expect_err("cross-channel reply must fail closed");

        assert_eq!(error.0, BrokerErrorCode::InvalidRequest);
        assert_eq!(submit_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn configured_mention_policy_does_not_block_edit_payloads() {
        let keys = Keys::generate();
        let required_recipient = Keys::generate().public_key();
        let event = EventBuilder::new(Kind::Custom(40003), "edited")
            .tags([])
            .sign_with_keys(&keys)
            .expect("edit event");
        let (mut rest, submit_count) = spawn_test_relay(
            None,
            true,
            serde_json::json!({
                "accepted": true,
                "event_id": event.id.to_hex(),
                "message": "stored"
            }),
        )
        .await;
        rest.keys = keys;

        submit_verified_message_with_policy(&rest, &event, Some(&required_recipient.to_hex()))
            .await
            .expect("edit remains deliverable");

        assert_eq!(submit_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn mention_policy_resolution_uses_parent_then_last_persona_value() {
        let first = Keys::generate().public_key().to_hex();
        let last = Keys::generate().public_key().to_hex();
        let parent = Keys::generate().public_key().to_hex();
        let persona = vec![
            (TOP_LEVEL_MENTION_PUBKEYS_ENV.into(), first),
            ("UNRELATED".into(), "value".into()),
            (TOP_LEVEL_MENTION_PUBKEYS_ENV.into(), last.clone()),
        ];

        assert_eq!(
            select_top_level_mention_policy(Some(parent.clone()), &persona),
            Some(parent)
        );
        assert_eq!(select_top_level_mention_policy(None, &persona), Some(last));
    }

    #[test]
    fn mention_policy_normalization_collapses_duplicates_and_validates() {
        let first = Keys::generate().public_key().to_hex();
        let selected = Keys::generate().public_key().to_hex();
        let mut persona = vec![
            (TOP_LEVEL_MENTION_PUBKEYS_ENV.into(), first),
            ("UNRELATED".into(), "value".into()),
            (TOP_LEVEL_MENTION_PUBKEYS_ENV.into(), selected.clone()),
        ];

        let result = normalize_top_level_mention_policy_with_parent(None, &mut persona)
            .expect("valid policy");

        assert_eq!(result, Some(selected.clone()));
        let normalized = persona
            .iter()
            .filter(|(key, _)| key == TOP_LEVEL_MENTION_PUBKEYS_ENV)
            .map(|(_, value)| value.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            normalized,
            vec![selected.as_str()],
            "normalization must leave one exact policy value"
        );

        let mut invalid = vec![(TOP_LEVEL_MENTION_PUBKEYS_ENV.into(), "invalid".into())];
        assert!(normalize_top_level_mention_policy_with_parent(None, &mut invalid).is_err());
    }

    #[tokio::test]
    async fn exact_preflight_reconciles_without_resubmitting() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(9), "already stored")
            .tags([])
            .sign_with_keys(&keys)
            .expect("event");
        let (mut rest, submit_count) = spawn_test_relay(
            Some(event.clone()),
            false,
            serde_json::json!({"accepted": false, "event_id": "wrong"}),
        )
        .await;
        rest.keys = keys;

        let result = submit_verified_message(&rest, &event)
            .await
            .expect("reconciled result");
        assert_eq!(result["event_id"], event.id.to_hex());
        assert_eq!(result["delivery_path"], "harness_broker");
        assert_eq!(result["readback_verified"], true);
        assert_eq!(result["reconciled"], true);
        assert_eq!(submit_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn rejected_receipt_is_reconciled_only_by_exact_readback() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(9), "stored despite receipt")
            .tags([])
            .sign_with_keys(&keys)
            .expect("event");
        let (mut rest, submit_count) = spawn_test_relay(
            None,
            true,
            serde_json::json!({
                "accepted": false,
                "event_id": "wrong",
                "message": "ambiguous rejection"
            }),
        )
        .await;
        rest.keys = keys;

        let result = submit_verified_message(&rest, &event)
            .await
            .expect("exact readback reconciles");
        assert_eq!(result["event_id"], event.id.to_hex());
        assert_eq!(result["delivery_path"], "harness_broker");
        assert_eq!(result["readback_verified"], true);
        assert_eq!(result["reconciled"], true);
        assert_eq!(submit_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn non_object_receipt_is_authoritative_after_exact_readback() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(9), "stored with malformed receipt")
            .tags([])
            .sign_with_keys(&keys)
            .expect("event");
        let (mut rest, submit_count) =
            spawn_test_relay(None, true, serde_json::json!("stored")).await;
        rest.keys = keys;

        let result = submit_verified_message(&rest, &event)
            .await
            .expect("exact readback is authoritative");
        assert_eq!(result["event_id"], event.id.to_hex());
        assert_eq!(result["accepted"], true);
        assert_eq!(result["delivery_path"], "harness_broker");
        assert_eq!(result["readback_verified"], true);
        assert_eq!(result["reconciled"], true);
        assert_eq!(submit_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn rejected_or_wrong_receipt_without_readback_fails_closed() {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(9), "not stored")
            .tags([])
            .sign_with_keys(&keys)
            .expect("event");
        let (mut rest, submit_count) = spawn_test_relay(
            None,
            false,
            serde_json::json!({
                "accepted": false,
                "event_id": "wrong",
                "message": "rejected"
            }),
        )
        .await;
        rest.keys = keys;

        let error = submit_verified_message(&rest, &event)
            .await
            .expect_err("missing readback must fail");
        assert_eq!(error.0, BrokerErrorCode::RelayRejected);
        assert_eq!(submit_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn capability_comparison_requires_exact_value() {
        assert!(capabilities_equal("abc", "abc"));
        assert!(!capabilities_equal("abc", "abd"));
        assert!(!capabilities_equal("abc", "abcd"));
    }

    #[cfg(unix)]
    #[test]
    fn request_reader_does_not_follow_symlinks() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let target = temp.path().join("target");
        let link = temp.path().join("request.json");
        std::fs::write(&target, b"secret").expect("target");
        symlink(&target, &link).expect("symlink");
        assert!(read_bounded_regular_file(&link, 1024).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn request_reader_does_not_follow_reparse_point_symlinks() {
        use std::os::windows::fs::symlink_file;

        let temp = tempfile::tempdir().expect("tempdir");
        let target = temp.path().join("target");
        let link = temp.path().join("request.json");
        std::fs::write(&target, b"secret").expect("target");
        if symlink_file(&target, &link).is_err() {
            // Windows requires Developer Mode or SeCreateSymbolicLinkPrivilege.
            return;
        }
        assert!(read_bounded_regular_file(&link, 1024).is_err());
    }

    #[test]
    fn invalid_queue_entry_is_removed_instead_of_rescanned() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(temp.path().join("requests")).expect("requests");
        std::fs::create_dir(temp.path().join("processing")).expect("processing");
        let invalid = temp.path().join("requests/not-a-request.json");
        std::fs::write(&invalid, b"bad").expect("invalid request");

        assert!(next_request_path(temp.path()).expect("scan").is_none());
        assert!(!invalid.exists());
    }

    #[test]
    fn in_progress_atomic_request_file_is_not_removed() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(temp.path().join("requests")).expect("requests");
        std::fs::create_dir(temp.path().join("processing")).expect("processing");
        let partial = temp.path().join("requests/.request.json.random.tmp");
        std::fs::write(&partial, b"partial").expect("partial request");

        assert!(next_request_path(temp.path()).expect("scan").is_none());
        assert!(partial.exists());
    }

    #[test]
    fn atomic_staging_clutter_does_not_starve_a_complete_request() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(temp.path().join("requests")).expect("requests");
        std::fs::create_dir(temp.path().join("processing")).expect("processing");
        for index in 0..MAX_SCAN_ENTRIES {
            std::fs::write(
                temp.path()
                    .join("requests")
                    .join(format!(".request-{index}.tmp")),
                b"partial",
            )
            .expect("staging file");
        }
        let request_id = Uuid::new_v4();
        std::fs::write(
            temp.path()
                .join("requests")
                .join(format!("{request_id}.json")),
            b"{}",
        )
        .expect("complete request");

        let claimed = next_request_path(temp.path())
            .expect("scan")
            .expect("complete request remains claimable");
        assert_eq!(claimed.0, request_id);
        assert_eq!(
            std::fs::read_dir(temp.path().join("requests"))
                .expect("requests")
                .count(),
            MAX_SCAN_ENTRIES
        );
    }

    #[test]
    fn saturated_broker_returns_a_correlated_signed_busy_response() {
        use std::collections::HashSet;

        let temp = tempfile::tempdir().expect("tempdir");
        for child in ["requests", "processing", "responses"] {
            std::fs::create_dir(temp.path().join(child)).expect("broker directory");
        }
        let request_ids: HashSet<Uuid> = (0..=MAX_CONCURRENT_REQUESTS)
            .map(|_| Uuid::new_v4())
            .collect();
        for request_id in &request_ids {
            std::fs::write(
                temp.path()
                    .join("requests")
                    .join(format!("{request_id}.json")),
                b"{}",
            )
            .expect("queued request");
        }

        let mut admitted = HashSet::new();
        for _ in 0..MAX_CONCURRENT_REQUESTS {
            let (request_id, _) = next_request_path(temp.path())
                .expect("scan")
                .expect("admitted request");
            admitted.insert(request_id);
        }
        let overflow_id = request_ids
            .difference(&admitted)
            .copied()
            .next()
            .expect("one overflow request");
        let response_keys = Keys::generate();
        reject_queued_requests(temp.path(), &response_keys);

        assert_eq!(
            std::fs::read_dir(temp.path().join("processing"))
                .expect("processing")
                .count(),
            MAX_CONCURRENT_REQUESTS,
            "busy response must not disturb admitted work"
        );
        assert_eq!(
            std::fs::read_dir(temp.path().join("requests"))
                .expect("requests")
                .count(),
            0,
            "overflow request must receive a response instead of timing out"
        );
        let bytes = std::fs::read(
            temp.path()
                .join("responses")
                .join(format!("{overflow_id}.json")),
        )
        .expect("busy response");
        let envelope: BrokerResponseEnvelope =
            serde_json::from_slice(&bytes).expect("signed response envelope");
        envelope.attestation.verify().expect("valid attestation");
        assert_eq!(envelope.attestation.pubkey, response_keys.public_key());
        assert_eq!(
            envelope.attestation.content,
            broker_response_digest(&envelope.response).expect("response digest")
        );
        assert_eq!(envelope.response.request_id, overflow_id);
        assert!(matches!(
            envelope.response.error,
            Some(buzz_core::delivery_broker::BrokerError {
                code: BrokerErrorCode::Busy,
                ..
            })
        ));
    }

    #[test]
    fn oversized_response_is_replaced_by_a_bounded_signed_failure() {
        let request_id = Uuid::new_v4();
        let response_keys = Keys::generate();
        let response = BrokerResponse::success(
            request_id,
            serde_json::json!({"payload": "x".repeat(MAX_BROKER_RESPONSE_BYTES as usize)}),
        );

        let bytes = encode_signed_response(response, &response_keys).expect("bounded envelope");
        assert!(bytes.len() as u64 <= MAX_BROKER_RESPONSE_BYTES);
        let envelope: BrokerResponseEnvelope =
            serde_json::from_slice(&bytes).expect("failure envelope");
        envelope.attestation.verify().expect("valid attestation");
        assert_eq!(envelope.attestation.pubkey, response_keys.public_key());
        assert_eq!(
            envelope.attestation.content,
            broker_response_digest(&envelope.response).expect("response digest")
        );
        assert_eq!(envelope.response.request_id, request_id);
        assert!(matches!(
            envelope.response.error,
            Some(buzz_core::delivery_broker::BrokerError {
                code: BrokerErrorCode::Internal,
                ..
            })
        ));
    }

    #[test]
    fn stale_root_predicates_require_age_and_proven_dead_process() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100_000);
        let old_root = now - LEGACY_ROOT_STALE_AGE - Duration::from_secs(1);

        assert!(legacy_root_is_stale(now, Some(old_root)));
        assert!(!legacy_root_is_stale(now, Some(now)));
        assert!(lease_root_is_reapable(true, ProcessLiveness::Dead));
        assert!(!lease_root_is_reapable(true, ProcessLiveness::Alive));
        assert!(!lease_root_is_reapable(true, ProcessLiveness::Unknown));
        assert!(!lease_root_is_reapable(false, ProcessLiveness::Dead));
    }

    #[cfg(unix)]
    #[test]
    fn current_process_is_not_classified_dead() {
        assert_ne!(process_liveness(std::process::id()), ProcessLiveness::Dead);
    }
}
