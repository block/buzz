#![deny(unsafe_code)]
#![warn(missing_docs)]
//! Shared durable whole-community deletion engine and store adapters.

#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use buzz_db::deletion::{
    ClaimedDeletion, DeletionRequest, DeletionStage, DeletionStore, FrozenInventory,
    KeyStreamDigest, LeaseToken, PrefixManifest, StorageManifest, DEFAULT_LEASE_DURATION,
};
use buzz_db::{Db, DbConfig};
use buzz_media::{is_tenant_owned_key, tenant_prefixes, MediaStorage};
use clap::Subcommand;
use serde::Serialize;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Fleet-wide object cap for one observational taxonomy sweep.
const DEFAULT_SWEEP_OBJECT_CAP: u64 = 10_000_000;
/// Keys per frozen side-table chunk (one `DeleteObjects`-sized unit × 10).
const DEFAULT_MANIFEST_CHUNK_KEYS: usize = 10_000;
/// Unknown keys retained verbatim on a sweep record for diagnosis.
const SWEEP_UNKNOWN_KEY_SAMPLE: usize = 100;
/// One S3 LIST page.
const LIST_PAGE_SIZE: usize = 1000;
const RETRY_DELAY: Duration = Duration::from_secs(30);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);

fn heartbeat_interval() -> Duration {
    HEARTBEAT_INTERVAL
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("deletion execution lease heartbeat failed")]
struct DeletionLeaseLost;

#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
struct ServingWriteLeaseLost {
    message: String,
}

/// Return the shared durable deletion store for relay and operator paths.
pub fn store(db: &Db) -> DeletionStore {
    db.deletion_store()
}

/// Durable, heartbeated lease for a serving-path external side effect.
pub struct ServingWriteGuard {
    store: DeletionStore,
    lease: buzz_db::deletion::ServingWriteLease,
    cancel: CancellationToken,
    lost: CancellationToken,
    finished: bool,
}

impl ServingWriteGuard {
    /// Verify this side-effect lease is still current before an irreversible call.
    pub async fn verify(&self) -> Result<()> {
        if self.lost.is_cancelled() {
            return Err(ServingWriteLeaseLost {
                message: "serving write lease heartbeat was lost".to_string(),
            }
            .into());
        }
        self.store
            .verify_serving_write_lease(&self.lease)
            .await
            .map_err(|error| ServingWriteLeaseLost {
                message: error.to_string(),
            })?;
        Ok(())
    }

    /// Run an external side effect while observing lease-heartbeat loss.
    ///
    /// Dropping the operation future on lease loss prevents a stale caller from
    /// continuing network I/O after its durable exclusion proof disappears.
    pub async fn protect<F, T>(&self, operation: F) -> Result<T>
    where
        F: std::future::Future<Output = T>,
    {
        self.verify().await?;
        let output = tokio::select! {
            biased;
            output = operation => output,
            _ = self.lost.cancelled() => {
                return Err(ServingWriteLeaseLost {
                    message: "serving write lease heartbeat was lost".to_string(),
                }
                .into())
            }
        };
        self.verify().await?;
        Ok(output)
    }

    /// Whether an error represents loss of a durable serving-write lease.
    pub fn is_lease_lost(error: &anyhow::Error) -> bool {
        error.downcast_ref::<ServingWriteLeaseLost>().is_some()
    }

    /// Whether a serving-write acquisition failed because the tenant is fenced.
    pub fn acquisition_is_fenced(error: &anyhow::Error) -> bool {
        matches!(
            error.downcast_ref::<buzz_db::DbError>(),
            Some(buzz_db::DbError::AccessDenied(_))
        )
    }

    /// Signal fired if the background lease heartbeat fails.
    pub fn lost(&self) -> CancellationToken {
        self.lost.clone()
    }

    /// The durable lease token presented to a final database mutation.
    pub fn lease(&self) -> &buzz_db::deletion::ServingWriteLease {
        &self.lease
    }

    /// Release the lease after the side effect completes.
    pub async fn finish(mut self) -> Result<()> {
        self.cancel.cancel();
        let released = self.store.release_serving_write_lease(&self.lease).await?;
        self.finished = true;
        if !released {
            return Err(ServingWriteLeaseLost {
                message: "serving write lease was already stale or released".to_string(),
            }
            .into());
        }
        Ok(())
    }
}

impl Drop for ServingWriteGuard {
    fn drop(&mut self) {
        self.cancel.cancel();
        if self.finished {
            return;
        }
        let store = self.store.clone();
        let lease = self.lease.clone();
        tokio::spawn(async move {
            let _ = store.release_serving_write_lease(&lease).await;
        });
    }
}

/// Acquire a serving-side external-effect lease without holding a pool connection.
///
/// A separate short database lease per effect is intentional: it is the only
/// durable proof that deletion can drain S3/Redis/push work across replicas.
/// PostgreSQL lease-table churn is reaped and exported by the relay pool-metrics
/// task; operators should watch the deletion lease gauges documented by Helm.
pub async fn acquire_serving_write(
    db: &Db,
    community: buzz_core::CommunityId,
    operation: &str,
) -> Result<ServingWriteGuard> {
    acquire_serving_write_with_heartbeat(db, community, operation, heartbeat_interval()).await
}

async fn acquire_serving_write_with_heartbeat(
    db: &Db,
    community: buzz_core::CommunityId,
    operation: &str,
    heartbeat_period: Duration,
) -> Result<ServingWriteGuard> {
    let store = store(db);
    let owner = default_executor_id();
    let lease = store
        .acquire_serving_write_lease(community, operation, &owner, DEFAULT_LEASE_DURATION)
        .await?;
    let heartbeat_store = store.clone();
    let mut heartbeat_lease = lease.clone();
    let cancel = CancellationToken::new();
    let heartbeat_cancel = cancel.clone();
    let lost = CancellationToken::new();
    let heartbeat_lost = lost.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(heartbeat_period);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            tokio::select! {
                _ = heartbeat_cancel.cancelled() => return,
                _ = interval.tick() => {
                    if heartbeat_store
                        .renew_serving_write_lease(
                            &mut heartbeat_lease,
                            DEFAULT_LEASE_DURATION,
                        )
                        .await
                        .is_err()
                    {
                        heartbeat_lost.cancel();
                        return;
                    }
                }
            }
        }
    });
    Ok(ServingWriteGuard {
        store,
        lease,
        cancel,
        lost,
        finished: false,
    })
}

/// CLI-only whole-community deletion commands.
#[derive(Subcommand)]
pub enum Command {
    /// Persist a deletion request and freeze its initial cross-store inventory.
    Submit {
        /// Canonical community host. Defaults to RELAY_URL's authority.
        #[arg(long)]
        host: Option<String>,
        /// Operator identity recorded on the request.
        #[arg(long)]
        requested_by: String,
        /// Optional reason for the request.
        #[arg(long)]
        reason: Option<String>,
    },
    /// List deletion requests as JSON.
    List {
        /// Maximum records.
        #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u16).range(1..=1000))]
        limit: u16,
    },
    /// Inspect one request, including approval/checkpoints/errors.
    Inspect {
        /// Deletion request UUID.
        id: Uuid,
    },
    /// Explicitly approve the exact frozen inventory digest.
    Approve {
        /// Deletion request UUID.
        id: Uuid,
        /// Approving operator identity.
        #[arg(long)]
        approved_by: String,
        /// Optional approval note.
        #[arg(long)]
        note: Option<String>,
    },
    /// Terminally cancel before irreversible object deletion begins.
    Abort {
        /// Deletion request UUID.
        id: Uuid,
        /// Aborting operator identity.
        #[arg(long)]
        aborted_by: String,
        /// Reason recorded in immutable audit evidence.
        #[arg(long)]
        reason: String,
    },
    /// Resume a blocked request after remediating its recorded failure.
    Unblock {
        /// Deletion request UUID.
        id: Uuid,
        /// Operator identity recorded in the recovery checkpoint.
        #[arg(long)]
        unblocked_by: String,
        /// Remediation or change reference recorded in the checkpoint.
        #[arg(long)]
        reason: String,
    },
    /// Claim and run one request until terminal/blocked.
    Run {
        /// Deletion request UUID.
        id: Uuid,
        /// Executor identity (defaults to hostname/pid).
        #[arg(long)]
        executor_id: Option<String>,
    },
    /// Drain the currently runnable deletion queue, then exit.
    Drain {
        /// Executor identity (defaults to hostname/pid).
        #[arg(long)]
        executor_id: Option<String>,
    },
    /// Sweep the whole bucket's key taxonomy and record observational evidence.
    ///
    /// This is independent of community deletion. It reports unknown writer
    /// shapes but never gates submission or destructive progress.
    Sweep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoopMode {
    Run,
    Drain,
}

impl LoopMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Drain => "drain",
        }
    }
}

#[derive(Clone)]
struct Services {
    store: DeletionStore,
    media: Arc<MediaStorage>,
    redis: deadpool_redis::Pool,
}

#[derive(Debug, thiserror::Error)]
enum EngineError {
    #[error("permanent deletion safety failure: {0}")]
    Permanent(String),
    #[error("transient deletion dependency failure: {0}")]
    Transient(String),
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
struct PermanentSource(#[from] anyhow::Error);

fn permanent(message: impl Into<String>) -> anyhow::Error {
    EngineError::Permanent(message.into()).into()
}

fn permanent_source(error: impl Into<anyhow::Error>) -> anyhow::Error {
    PermanentSource(error.into()).into()
}

fn transient(message: impl Into<String>) -> anyhow::Error {
    EngineError::Transient(message.into()).into()
}

fn is_permanent_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.is::<PermanentSource>()
            || matches!(
                cause.downcast_ref::<buzz_db::DbError>(),
                Some(buzz_db::DbError::DeletionSafety(_))
            )
            || cause
                .downcast_ref::<EngineError>()
                .is_some_and(|error| matches!(error, EngineError::Permanent(_)))
    })
}

#[derive(Debug, Serialize)]
struct RunOutput {
    request_id: Uuid,
    stage: DeletionStage,
    retry_count: i32,
    last_error: Option<String>,
    next_attempt_at: chrono::DateTime<chrono::Utc>,
    blocked_reason: Option<String>,
}

/// Execute one nested deletion command.
pub async fn run(command: Command) -> Result<i32> {
    match command {
        Command::List { limit } => {
            let store = connect_store().await?;
            print_json(&store.list(i64::from(limit)).await?)?;
            Ok(0)
        }
        Command::Inspect { id } => {
            let store = connect_store().await?;
            print_json(&store.inspect(id).await?)?;
            Ok(0)
        }
        Command::Approve {
            id,
            approved_by,
            note,
        } => {
            let store = connect_store().await?;
            print_json(&store.approve(id, &approved_by, note.as_deref()).await?)?;
            Ok(0)
        }
        Command::Abort {
            id,
            aborted_by,
            reason,
        } => {
            let store = connect_store().await?;
            print_json(&store.abort(id, &aborted_by, &reason).await?)?;
            Ok(0)
        }
        Command::Unblock {
            id,
            unblocked_by,
            reason,
        } => {
            let store = connect_store().await?;
            print_json(&store.unblock(id, &unblocked_by, &reason).await?)?;
            Ok(0)
        }
        Command::Run { id, executor_id } => {
            let store = connect_store().await?;
            let services = match connect_services_with_store(store.clone()).await {
                Ok(services) => services,
                Err(error) => {
                    let message = format!("{error:#}");
                    store
                        .block_preclaim_setup(id, "pre_claim:service_setup", &message)
                        .await
                        .context("record pre-claim service setup failure")?;
                    return Err(error);
                }
            };
            run_loop(
                services,
                LoopMode::Run,
                Some(id),
                executor_id.unwrap_or_else(default_executor_id),
            )
            .await
        }
        command => run_with_services(command, connect_services().await?).await,
    }
}

async fn run_with_services(command: Command, services: Services) -> Result<i32> {
    match command {
        Command::Submit {
            host,
            requested_by,
            reason,
        } => {
            let relay_url = std::env::var("RELAY_URL").ok();
            let host = resolve_submit_host(host.as_deref(), relay_url.as_deref())?;
            let request = services
                .store
                .submit(&host, &requested_by, reason.as_deref())
                .await?;
            let inventory = build_inventory(&services, &request).await?;
            let request = services
                .store
                .freeze_inventory(request.id, &inventory)
                .await?;
            print_json(&request)?;
            Ok(0)
        }
        Command::Run { id, executor_id } => {
            run_loop(
                services,
                LoopMode::Run,
                Some(id),
                executor_id.unwrap_or_else(default_executor_id),
            )
            .await
        }
        Command::Drain { executor_id } => {
            run_loop(
                services,
                LoopMode::Drain,
                None,
                executor_id.unwrap_or_else(default_executor_id),
            )
            .await
        }
        Command::Sweep => {
            let started_at = chrono::Utc::now();
            let cap = sweep_object_cap();
            let media = Arc::clone(&services.media);
            let outcome =
                buzz_media::sweep_bucket_taxonomy(cap, SWEEP_UNKNOWN_KEY_SAMPLE, move |token| {
                    let media = Arc::clone(&media);
                    async move { media.list_page(token, LIST_PAGE_SIZE).await }
                })
                .await?;
            let sweep = services
                .store
                .record_taxonomy_sweep(
                    started_at,
                    outcome.listed_objects,
                    outcome.unknown_object_count,
                    &outcome.unknown_key_sample,
                    cap,
                )
                .await?;
            print_json(&sweep)?;
            Ok(i32::from(sweep.unknown_object_count > 0))
        }
        Command::List { .. }
        | Command::Inspect { .. }
        | Command::Approve { .. }
        | Command::Abort { .. }
        | Command::Unblock { .. } => {
            anyhow::bail!("database-only command reached full-service dispatcher")
        }
    }
}

fn resolve_submit_host(host: Option<&str>, relay_url: Option<&str>) -> Result<String> {
    if let Some(host) = host {
        let host = host.trim();
        if host.is_empty() {
            anyhow::bail!("--host must not be empty");
        }
        return Ok(host.to_owned());
    }

    let relay_url = relay_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("cannot derive community host; pass --host or set RELAY_URL")
        })?;
    let host = buzz_core::tenant::relay_url_authority(relay_url);
    if host.is_empty() {
        anyhow::bail!(
            "cannot derive community host from RELAY_URL; pass --host or set a valid RELAY_URL"
        );
    }
    Ok(host)
}

async fn connect_store() -> Result<DeletionStore> {
    let database_url = required_env("DATABASE_URL")?;
    let db = Db::new(&DbConfig {
        database_url,
        max_connections: env_parse("BUZZ_DB_POOL_SIZE", 20),
        ..DbConfig::default()
    })
    .await?;
    Ok(store(&db))
}

fn resolve_s3_region(buzz_region: Option<String>, aws_region: Option<String>) -> String {
    buzz_region
        .and_then(nonempty_s3_region)
        .or_else(|| aws_region.and_then(nonempty_s3_region))
        .unwrap_or_else(|| "us-east-1".to_string())
}

fn nonempty_s3_region(region: String) -> Option<String> {
    let region = region.trim();
    (!region.is_empty()).then(|| region.to_string())
}

fn s3_region_from_env() -> String {
    resolve_s3_region(
        std::env::var("BUZZ_S3_REGION").ok(),
        std::env::var("AWS_REGION").ok(),
    )
}

async fn connect_services() -> Result<Services> {
    let store = connect_store().await?;
    connect_services_with_store(store).await
}

async fn connect_services_with_store(store: DeletionStore) -> Result<Services> {
    let media_config = buzz_media::MediaConfig {
        s3_endpoint: required_env("BUZZ_S3_ENDPOINT")?,
        s3_access_key: required_env("BUZZ_S3_ACCESS_KEY")?,
        s3_secret_key: required_env("BUZZ_S3_SECRET_KEY")?,
        s3_bucket: required_env("BUZZ_S3_BUCKET")?,
        s3_region: s3_region_from_env(),
        s3_addressing_style: std::env::var("BUZZ_S3_ADDRESSING_STYLE")
            .unwrap_or_else(|_| "path".to_string())
            .parse()
            .map_err(anyhow::Error::msg)?,
        max_image_bytes: 1,
        max_gif_bytes: 1,
        max_video_bytes: 1,
        max_file_bytes: 1,
        public_base_url: "http://localhost/media".to_string(),
        upload_records_enabled: false,
        upload_ip_header: None,
        upload_port_header: None,
    };
    let media = Arc::new(MediaStorage::new(&media_config)?);
    let redis_url = required_env("REDIS_URL")?;
    let mut redis_config = deadpool_redis::Config::from_url(&redis_url);
    redis_config.pool = Some(deadpool_redis::PoolConfig::new(env_parse(
        "BUZZ_REDIS_POOL_SIZE",
        16,
    )));
    let redis = redis_config
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .context("create deletion Redis pool")?;
    Ok(Services {
        store,
        media,
        redis,
    })
}

fn required_env(name: &str) -> Result<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("{name} is required for community deletion"))
}

fn env_parse<T>(name: &str, default: T) -> T
where
    T: std::str::FromStr,
{
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn validate_frozen_inventory(request: &DeletionRequest) -> Result<FrozenInventory> {
    let frozen: FrozenInventory = serde_json::from_value(
        request
            .inventory_manifest
            .clone()
            .ok_or_else(|| permanent("approved request has no frozen inventory"))?,
    )
    .map_err(permanent_source)?;
    let expected_digest = request
        .inventory_digest
        .as_deref()
        .ok_or_else(|| permanent("approved request has no frozen inventory digest"))?;
    let actual_digest = hex::encode(frozen.digest().map_err(permanent_source)?);
    if actual_digest != expected_digest {
        return Err(permanent("approved frozen inventory digest mismatch"));
    }
    validate_storage_ownership(request, &frozen.storage)?;
    Ok(frozen)
}

fn validate_storage_ownership(request: &DeletionRequest, manifest: &StorageManifest) -> Result<()> {
    buzz_db::deletion::validate_storage_manifest(manifest)?;
    let expected = tenant_prefixes(*request.community_id.as_uuid());
    let actual = manifest
        .prefixes
        .iter()
        .map(|prefix| prefix.prefix.as_str())
        .collect::<Vec<_>>();
    if actual != expected.iter().map(String::as_str).collect::<Vec<_>>() {
        return Err(permanent(
            "storage manifest prefixes are not the deletion target's tenant prefixes",
        ));
    }
    Ok(())
}

async fn build_inventory(
    services: &Services,
    request: &DeletionRequest,
) -> Result<FrozenInventory> {
    let schema = services
        .store
        .inventory_schema(request.community_id)
        .await?;
    let storage = enumerate_tenant_prefixes(services, request, None, None).await?;
    Ok(FrozenInventory { schema, storage })
}

/// Buffered writer for frozen key chunks during the destructive freeze.
struct ChunkSink<'a> {
    token: &'a LeaseToken,
    next_chunk_no: i64,
    buffered: Vec<String>,
}

async fn flush_chunk(services: &Services, sink: &mut ChunkSink<'_>, prefix: &str) -> Result<()> {
    if sink.buffered.is_empty() {
        return Ok(());
    }
    services
        .store
        .append_manifest_key_chunk(sink.token, sink.next_chunk_no, prefix, &sink.buffered)
        .await?;
    sink.next_chunk_no += 1;
    sink.buffered.clear();
    Ok(())
}

/// Enumerate the target's three tenant prefixes into per-prefix summaries.
///
/// Cost is O(tenant objects) regardless of fleet size. Unknown shapes inside
/// one of the owned prefixes fail closed; keys elsewhere in the shared bucket
/// are outside this operation's contract. Memory stays bounded at one listing
/// page plus one buffered chunk — the full key list is never materialized.
/// When `sink` is supplied, keys are also persisted as side-table chunks
/// (never spanning prefixes) for the destructive freeze to bind against these
/// digests.
async fn enumerate_tenant_prefixes(
    services: &Services,
    request: &DeletionRequest,
    heartbeat_lost: Option<&CancellationToken>,
    mut sink: Option<&mut ChunkSink<'_>>,
) -> Result<StorageManifest> {
    if services.media.bucket_versioning_detected().await? {
        return Err(permanent(
            "bucket versioning detected; deletion cannot prove logical absence with delete markers",
        ));
    }
    let community = *request.community_id.as_uuid();
    let chunk_keys = manifest_chunk_keys();
    let mut prefixes = Vec::new();
    for prefix in tenant_prefixes(community) {
        let mut digest = KeyStreamDigest::new();
        let mut total_bytes: u64 = 0;
        let mut continuation = None;
        loop {
            if heartbeat_lost.is_some_and(CancellationToken::is_cancelled) {
                return Err(DeletionLeaseLost.into());
            }
            let page = services
                .media
                .list_prefix_page(&prefix, continuation.take(), LIST_PAGE_SIZE)
                .await?;
            for (key, size) in page.objects {
                if !is_tenant_owned_key(community, &key) {
                    return Err(permanent(format!(
                        "key under a tenant prefix is outside the exact writer taxonomy: {key}"
                    )));
                }
                digest.fold(&key)?;
                total_bytes = total_bytes.saturating_add(size);
                if let Some(sink) = sink.as_deref_mut() {
                    sink.buffered.push(key);
                    if sink.buffered.len() >= chunk_keys {
                        flush_chunk(services, sink, &prefix).await?;
                    }
                }
            }
            if !page.is_truncated {
                break;
            }
            continuation = page.next_continuation_token;
            if continuation.is_none() {
                return Err(transient(
                    "truncated tenant listing page has no continuation token",
                ));
            }
        }
        if let Some(sink) = sink.as_deref_mut() {
            flush_chunk(services, sink, &prefix).await?;
        }
        let (keys_digest, object_count) = digest.finish();
        prefixes.push(PrefixManifest {
            prefix,
            object_count,
            total_bytes,
            keys_digest,
        });
    }
    let manifest = StorageManifest {
        version: 4,
        prefixes,
    };
    buzz_db::deletion::validate_storage_manifest(&manifest)?;
    Ok(manifest)
}

/// Freeze the post-fence, post-drain destructive enumeration: stream the
/// tenant prefixes into side-table chunks, then bind the chunk stream to the
/// request row's digests atomically.
async fn freeze_destructive_manifest(
    services: &Services,
    request: &DeletionRequest,
    token: &LeaseToken,
    heartbeat_lost: &CancellationToken,
) -> Result<StorageManifest> {
    validate_frozen_inventory(request)?;
    // A prior interrupted freeze may have left partial chunks; they were
    // never bound to a committed manifest, so rewrite them from scratch.
    services.store.clear_manifest_key_chunks(token).await?;
    let mut sink = ChunkSink {
        token,
        next_chunk_no: 0,
        buffered: Vec::new(),
    };
    let manifest =
        enumerate_tenant_prefixes(services, request, Some(heartbeat_lost), Some(&mut sink)).await?;
    services
        .store
        .freeze_destructive_storage_manifest(token, &manifest)
        .await?;
    Ok(manifest)
}

async fn run_loop(
    services: Services,
    mode: LoopMode,
    request_id: Option<Uuid>,
    executor_id: String,
) -> Result<i32> {
    let shutdown = shutdown_token();
    let mut ran = false;
    loop {
        if shutdown.is_cancelled() {
            services
                .store
                .stop_executor(None, &executor_id)
                .await
                .context("record executor drain")?;
            return Ok(0);
        }
        let claim = match request_id {
            Some(id) => {
                services
                    .store
                    .claim_specific(id, &executor_id, DEFAULT_LEASE_DURATION)
                    .await?
            }
            None => {
                services
                    .store
                    .claim_next(&executor_id, DEFAULT_LEASE_DURATION)
                    .await?
            }
        };
        let Some(claim) = claim else {
            if mode == LoopMode::Run && !ran {
                anyhow::bail!(
                    "deletion request is not runnable, is blocked, or is leased by another executor"
                );
            }
            return Ok(0);
        };
        ran = true;
        let output = execute_claim(&services, mode, claim, &shutdown).await?;
        print_json(&output)?;
        let failed = output.last_error.is_some() || output.blocked_reason.is_some();
        if mode == LoopMode::Run || shutdown.is_cancelled() || failed {
            return Ok(i32::from(failed));
        }
    }
}

async fn stop_claim_executor(
    services: &Services,
    mode: LoopMode,
    token: &LeaseToken,
) -> Result<()> {
    // A failed draining heartbeat must not prevent the generation-checked release
    // attempt. `stop_executor` cannot clear a successor's reclaimed lease.
    let _ = services
        .store
        .heartbeat(token, mode.as_str(), DEFAULT_LEASE_DURATION, true)
        .await;
    services
        .store
        .stop_executor(Some(token), &token.owner)
        .await?;
    Ok(())
}

async fn record_stage_failure(
    services: &Services,
    token: &LeaseToken,
    stage: DeletionStage,
    error: &anyhow::Error,
) -> Result<bool> {
    let message = format!("{error:#}");
    let result = if is_permanent_error(error) {
        services.store.block(token, stage, "stage", &message).await
    } else {
        services
            .store
            .record_retry(token, stage, "stage", &message, RETRY_DELAY)
            .await
    };
    match result {
        Ok(()) => Ok(true),
        Err(error) if buzz_db::deletion::is_stale_deletion_lease(&error) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

async fn execute_claim(
    services: &Services,
    mode: LoopMode,
    mut claim: ClaimedDeletion,
    shutdown: &CancellationToken,
) -> Result<RunOutput> {
    let token = claim.lease.clone();
    loop {
        if shutdown.is_cancelled() {
            stop_claim_executor(services, mode, &token).await?;
            let request = services.store.get(token.request_id).await?;
            return Ok(run_output(request));
        }
        services
            .store
            .heartbeat(&token, mode.as_str(), DEFAULT_LEASE_DURATION, false)
            .await?;
        let stage_result = run_stage_with_heartbeat(services, mode, &claim, shutdown).await;
        match stage_result {
            StageOutcome::Completed => {}
            StageOutcome::Shutdown => {
                stop_claim_executor(services, mode, &token).await?;
                let request = services.store.get(token.request_id).await?;
                return Ok(run_output(request));
            }
            StageOutcome::Failed(error) => {
                let request = services.store.get(token.request_id).await?;
                if request.lease_owner.as_deref() != Some(&token.owner)
                    || request.lease_generation != token.generation
                {
                    return Ok(run_output(request));
                }
                if !record_stage_failure(services, &token, claim.request.stage, &error).await? {
                    // Ownership expired between the stage failure and durable
                    // error recording. A successor now owns retry/block policy.
                    let request = services.store.get(token.request_id).await?;
                    return Ok(run_output(request));
                }
                let request = services.store.get(token.request_id).await?;
                return Ok(run_output(request));
            }
        }
        let request = services.store.get(token.request_id).await?;
        if request.stage == DeletionStage::RetentionPending || request.blocked_reason.is_some() {
            return Ok(run_output(request));
        }
        claim.request = request.clone();
        claim.lease.fence_generation = request.fence_generation;
    }
}

enum StageOutcome {
    Completed,
    Shutdown,
    Failed(anyhow::Error),
}

async fn await_stage<F>(
    stage: F,
    shutdown: &CancellationToken,
    heartbeat_error: &CancellationToken,
) -> StageOutcome
where
    F: std::future::Future<Output = Result<()>>,
{
    tokio::select! {
        biased;
        _ = shutdown.cancelled() => StageOutcome::Shutdown,
        _ = heartbeat_error.cancelled() => StageOutcome::Failed(DeletionLeaseLost.into()),
        result = stage => match result {
            Ok(()) => StageOutcome::Completed,
            Err(error) => StageOutcome::Failed(error),
        },
    }
}

async fn run_stage_with_heartbeat(
    services: &Services,
    mode: LoopMode,
    claim: &ClaimedDeletion,
    shutdown: &CancellationToken,
) -> StageOutcome {
    let heartbeat_services = services.clone();
    let heartbeat_token = claim.lease.clone();
    let heartbeat_mode = mode.as_str();
    let heartbeat_shutdown = CancellationToken::new();
    let heartbeat_cancel = heartbeat_shutdown.clone();
    let heartbeat_error = CancellationToken::new();
    let heartbeat_error_signal = heartbeat_error.clone();
    let heartbeat = tokio::spawn(async move {
        let mut interval = tokio::time::interval(heartbeat_interval());
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            tokio::select! {
                _ = heartbeat_cancel.cancelled() => return,
                _ = interval.tick() => {
                    if heartbeat_services
                        .store
                        .heartbeat(
                            &heartbeat_token,
                            heartbeat_mode,
                            DEFAULT_LEASE_DURATION,
                            false,
                        )
                        .await
                        .is_err()
                    {
                        heartbeat_error_signal.cancel();
                        return;
                    }
                }
            }
        }
    });

    let stage = await_stage(
        execute_stage(services, claim, &heartbeat_error),
        shutdown,
        &heartbeat_error,
    )
    .await;
    heartbeat_shutdown.cancel();
    match heartbeat.await {
        Ok(()) => stage,
        Err(error) => {
            StageOutcome::Failed(anyhow::anyhow!("deletion heartbeat task failed: {error}"))
        }
    }
}

async fn run_guarded_external_step<F, Fut, T>(
    services: &Services,
    token: &LeaseToken,
    stage: DeletionStage,
    heartbeat_lost: &CancellationToken,
    operation: F,
) -> Result<T>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    services.store.verify_execution_token(token, stage).await?;
    let result = tokio::select! {
        biased;
        _ = heartbeat_lost.cancelled() => {
            return Err(DeletionLeaseLost.into());
        }
        result = operation() => result,
    };
    let output = result?;
    services.store.verify_execution_token(token, stage).await?;
    Ok(output)
}

async fn execute_stage(
    services: &Services,
    claim: &ClaimedDeletion,
    heartbeat_lost: &CancellationToken,
) -> Result<()> {
    let request = &claim.request;
    let token = token_with_current_fence(&claim.lease, request);
    if matches!(
        request.stage,
        DeletionStage::Approved
            | DeletionStage::Fenced
            | DeletionStage::Drained
            | DeletionStage::BindingsRemoved
            | DeletionStage::PostgresPurged
            | DeletionStage::CachePurged
            | DeletionStage::LogicallyVerified
    ) {
        validate_frozen_inventory(request)?;
    }
    match request.stage {
        DeletionStage::Approved => {
            // Approval binds immutable catalog + community-prefix ownership.
            // Live row counts and tenant binding keys are deliberately not
            // equality-bound until the durable fence closes all writers.
            let live_schema = services
                .store
                .inventory_schema(request.community_id)
                .await?;
            let frozen = validate_frozen_inventory(request)?;
            if live_schema.scoped_tables != frozen.schema.scoped_tables
                || live_schema.fenced_tables != frozen.schema.fenced_tables
            {
                return Err(permanent(
                    "approved structural catalog drifted before fencing",
                ));
            }
            services.store.begin_quiescing(&token).await?;
            match services.store.fence(&token).await {
                Ok(_) => {}
                Err(buzz_db::DbError::ServingWritesNotDrained {
                    active_count,
                    operations,
                    ..
                }) => {
                    return Err(transient(format!(
                        "serving writes not drained before fence: count={active_count}, operations={operations:?}"
                    )));
                }
                Err(error) => return Err(error.into()),
            }
        }
        DeletionStage::Fenced => {
            services
                .store
                .verify_execution_token(&token, DeletionStage::Fenced)
                .await?;
            let disconnect = tokio::select! {
                biased;
                _ = heartbeat_lost.cancelled() => Err(DeletionLeaseLost.into()),
                result = publish_disconnect_community(&services.redis, request.community_id) => result,
            };
            disconnect?;
            services
                .store
                .verify_execution_token(&token, DeletionStage::Fenced)
                .await?;
            if !services
                .store
                .serving_writes_drained(request.community_id)
                .await?
            {
                return Err(transient("serving writes have not drained"));
            }
            // Freeze the destructive enumeration only after the fence closed
            // new writers AND every admitted serving write drained: the
            // post-drain listing is the final storage state, so no tenant key
            // can appear after the freeze.
            let destructive = match request.destructive_storage_manifest.clone() {
                Some(value) => serde_json::from_value(value)?,
                None => {
                    freeze_destructive_manifest(services, request, &token, heartbeat_lost).await?
                }
            };
            validate_storage_ownership(request, &destructive)?;
            services.store.mark_drained(&token).await?;
        }
        DeletionStage::Drained => {
            let storage: StorageManifest = serde_json::from_value(
                request
                    .destructive_storage_manifest
                    .clone()
                    .context("request has no post-fence destructive storage manifest")?,
            )?;
            validate_storage_ownership(request, &storage)?;
            // Resume = first unstamped chunk. Bulk deletes are idempotent
            // (missing keys report as deleted), so re-deleting a chunk whose
            // stamp was lost to a crash is safe.
            let mut removed: u64 = 0;
            let mut already_missing: u64 = 0;
            while let Some(chunk) = services.store.next_pending_manifest_chunk(&token).await? {
                let outcome = run_guarded_external_step(
                    services,
                    &token,
                    DeletionStage::Drained,
                    heartbeat_lost,
                    || async { Ok(services.media.delete_objects(&chunk.keys).await?) },
                )
                .await?;
                if !outcome.versioned_keys.is_empty() {
                    return Err(permanent(format!(
                        "bulk delete produced version artifacts; bucket versioning blocks \
                         deletion: {}",
                        outcome.versioned_keys.join(",")
                    )));
                }
                if !outcome.failed.is_empty() {
                    let (key, code, message) = &outcome.failed[0];
                    return Err(transient(format!(
                        "bulk delete failed for {} key(s); first: {key}: {code}: {message}",
                        outcome.failed.len()
                    )));
                }
                let acknowledged = outcome.deleted.saturating_add(outcome.already_missing);
                if acknowledged != chunk.keys.len() as u64 {
                    return Err(transient(format!(
                        "bulk delete acknowledged {acknowledged} of {} keys in chunk {}",
                        chunk.keys.len(),
                        chunk.chunk_no
                    )));
                }
                removed += outcome.deleted;
                already_missing += outcome.already_missing;
                services
                    .store
                    .mark_manifest_chunk_deleted(
                        &token,
                        chunk.chunk_no,
                        serde_json::json!({
                            "prefix": chunk.prefix,
                            "keys": chunk.keys.len(),
                            "deleted": outcome.deleted,
                            "already_missing": outcome.already_missing,
                        }),
                    )
                    .await?;
            }
            let frozen_keys: u64 = storage
                .prefixes
                .iter()
                .map(|prefix| prefix.object_count)
                .sum();
            services
                .store
                .mark_bindings_removed(
                    &token,
                    serde_json::json!({
                        "deleted_keys": frozen_keys,
                        "removed_now": removed,
                        "already_missing": already_missing,
                    }),
                )
                .await?;
        }
        DeletionStage::BindingsRemoved => {
            services.store.purge_postgres(&token).await?;
        }
        DeletionStage::PostgresPurged => {
            services
                .store
                .verify_execution_token(&token, DeletionStage::PostgresPurged)
                .await?;
            let deleted = purge_redis_namespace(&services.redis, request.community_id).await?;
            services
                .store
                .verify_execution_token(&token, DeletionStage::PostgresPurged)
                .await?;
            services
                .store
                .mark_cache_purged(&token, serde_json::json!({"deleted_keys": deleted}))
                .await?;
        }
        DeletionStage::CachePurged => {
            services
                .store
                .verify_postgres_logically_deleted(&token)
                .await?;
            verify_storage_absence(services, request).await?;
            verify_redis_absence(&services.redis, request.community_id).await?;
            services
                .store
                .mark_logically_verified(
                    &token,
                    serde_json::json!({"postgres": true, "object_store": true, "redis": true}),
                )
                .await?;
        }
        DeletionStage::LogicallyVerified => {
            validate_frozen_inventory(request)?;
            services
                .store
                .mark_retention_pending(
                    &token,
                    serde_json::json!({
                        "policy": "member-erasure and fleet-wide shared-CAS GC are out of V1 scope"
                    }),
                )
                .await?;
        }
        DeletionStage::Submitted | DeletionStage::Inventoried => {
            anyhow::bail!("request has not crossed the explicit approval boundary")
        }
        DeletionStage::RetentionPending | DeletionStage::Aborted => {}
    }
    Ok(())
}

fn token_with_current_fence(token: &LeaseToken, request: &DeletionRequest) -> LeaseToken {
    LeaseToken {
        fence_generation: request.fence_generation,
        ..token.clone()
    }
}

/// Prove logical absence by listing each tenant prefix and requiring it
/// empty — O(1) requests per prefix, independent of fleet size.
async fn verify_storage_absence(services: &Services, request: &DeletionRequest) -> Result<()> {
    for prefix in tenant_prefixes(*request.community_id.as_uuid()) {
        let page = services.media.list_prefix_page(&prefix, None, 1).await?;
        if let Some((key, _)) = page.objects.first() {
            return Err(transient(format!(
                "logical verification found a live target object binding: {key}"
            )));
        }
    }
    Ok(())
}

async fn publish_disconnect_community(
    pool: &deadpool_redis::Pool,
    community: buzz_core::CommunityId,
) -> Result<()> {
    let mut connection = pool.get().await?;
    let channel = format!("buzz:{community}:conn-control");
    let _: u64 = redis::cmd("PUBLISH")
        .arg(channel)
        .arg(r#"{"op":"DisconnectCommunity"}"#)
        .query_async(&mut *connection)
        .await?;
    Ok(())
}

async fn purge_redis_namespace(
    pool: &deadpool_redis::Pool,
    community: buzz_core::CommunityId,
) -> Result<u64> {
    let mut connection = pool.get().await?;
    let pattern = format!("buzz:{community}:*");
    let mut cursor = 0u64;
    let mut deleted = 0u64;
    loop {
        let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(&pattern)
            .arg("COUNT")
            .arg(1000)
            .query_async(&mut *connection)
            .await?;
        if !keys.is_empty() {
            let count: u64 = redis::cmd("UNLINK")
                .arg(&keys)
                .query_async(&mut *connection)
                .await?;
            deleted = deleted.saturating_add(count);
        }
        if next == 0 {
            break;
        }
        cursor = next;
    }
    Ok(deleted)
}

fn scan_proves_absence(pages: &[(u64, Vec<String>)]) -> bool {
    pages.last().is_some_and(|(cursor, _)| *cursor == 0)
        && pages.iter().all(|(_, keys)| keys.is_empty())
}

async fn scan_redis_namespace(
    connection: &mut deadpool_redis::Connection,
    pattern: &str,
) -> Result<Vec<(u64, Vec<String>)>> {
    let mut cursor = 0u64;
    let mut pages = Vec::new();
    loop {
        let page: (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(pattern)
            .arg("COUNT")
            .arg(1000)
            .query_async(&mut **connection)
            .await?;
        cursor = page.0;
        pages.push(page);
        if cursor == 0 {
            return Ok(pages);
        }
    }
}

async fn verify_redis_absence(
    pool: &deadpool_redis::Pool,
    community: buzz_core::CommunityId,
) -> Result<()> {
    let mut connection = pool.get().await?;
    let pattern = format!("buzz:{community}:*");
    // SCAN is weakly consistent. Two complete empty passes ensure a cursor
    // rollover or concurrent expiry cannot make one sparse pass look absent.
    let first = scan_redis_namespace(&mut connection, &pattern).await?;
    let second = scan_redis_namespace(&mut connection, &pattern).await?;
    if scan_proves_absence(&first) && scan_proves_absence(&second) {
        Ok(())
    } else {
        Err(transient(
            "logical verification found a Redis namespace key",
        ))
    }
}

fn sweep_object_cap() -> u64 {
    std::env::var("BUZZ_DELETION_SWEEP_MAX_OBJECTS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_SWEEP_OBJECT_CAP)
}

fn manifest_chunk_keys() -> usize {
    std::env::var("BUZZ_DELETION_MANIFEST_CHUNK_KEYS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .map(|value| value.min(100_000))
        .unwrap_or(DEFAULT_MANIFEST_CHUNK_KEYS)
}

fn default_executor_id() -> String {
    let hostname = std::env::var("HOSTNAME").unwrap_or_else(|_| "buzz-admin".to_string());
    format!("{hostname}:{}", std::process::id())
}

fn shutdown_token() -> CancellationToken {
    let token = CancellationToken::new();
    let signal = token.clone();
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal as unix_signal, SignalKind};
            if let Ok(mut terminate) = unix_signal(SignalKind::terminate()) {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {},
                    _ = terminate.recv() => {},
                }
            } else {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        signal.cancel();
    });
    token
}

fn run_output(request: DeletionRequest) -> RunOutput {
    RunOutput {
        request_id: request.id,
        stage: request.stage,
        retry_count: request.retry_count,
        last_error: request.last_error,
        next_attempt_at: request.next_attempt_at,
        blocked_reason: request.blocked_reason,
    }
}

fn print_json(value: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submit_host_prefers_explicit_host() {
        assert_eq!(
            resolve_submit_host(Some(" community.example "), Some("wss://ignored.example"))
                .expect("explicit host"),
            "community.example"
        );
    }

    #[test]
    fn submit_host_derives_from_relay_url() {
        assert_eq!(
            resolve_submit_host(None, Some("wss://relay.example:8443/path"))
                .expect("relay URL host"),
            "relay.example:8443"
        );
    }

    #[test]
    fn submit_host_requires_an_explicit_source() {
        for relay_url in [None, Some(""), Some("   ")] {
            let error = resolve_submit_host(None, relay_url).expect_err("missing host must fail");
            assert!(error.to_string().contains("pass --host or set RELAY_URL"));
        }
    }

    #[test]
    fn submit_host_rejects_empty_or_invalid_values() {
        assert!(resolve_submit_host(Some("   "), Some("wss://relay.example")).is_err());
        assert!(resolve_submit_host(None, Some("not a URL")).is_err());
    }

    fn empty_storage_manifest(community: buzz_core::CommunityId) -> StorageManifest {
        StorageManifest {
            version: 4,
            prefixes: tenant_prefixes(*community.as_uuid())
                .into_iter()
                .map(|prefix| PrefixManifest {
                    prefix,
                    object_count: 0,
                    total_bytes: 0,
                    keys_digest: KeyStreamDigest::new().finish().0,
                })
                .collect(),
        }
    }

    async fn claimed_test_deletion(prefix: &str) -> (Db, Services, ClaimedDeletion) {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .expect("BUZZ_TEST_DATABASE_URL or DATABASE_URL is required");
        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("connect deletion engine test DB");
        let db = Db::from_pool(pool);
        db.migrate().await.expect("migrate deletion engine test DB");
        let store = db.deletion_store();
        let host = format!("{prefix}-{}.example", Uuid::new_v4().simple());
        let community = db
            .ensure_configured_community(&host)
            .await
            .expect("create deletion engine test community");
        let request = store
            .submit(&host, "test", None)
            .await
            .expect("submit deletion request");
        let inventory = FrozenInventory {
            schema: store
                .inventory_schema(community.id)
                .await
                .expect("inventory schema"),
            storage: empty_storage_manifest(community.id),
        };
        store
            .freeze_inventory(request.id, &inventory)
            .await
            .expect("freeze deletion inventory");
        store
            .approve(request.id, "test", None)
            .await
            .expect("approve deletion request");
        let claim = store
            .claim_specific(request.id, "test-executor", DEFAULT_LEASE_DURATION)
            .await
            .expect("claim deletion request")
            .expect("runnable deletion request");
        let services = Services {
            store,
            media: Arc::new(
                MediaStorage::new(&buzz_media::MediaConfig {
                    s3_endpoint: "http://127.0.0.1:1".to_string(),
                    s3_access_key: "unused".to_string(),
                    s3_secret_key: "unused".to_string(),
                    s3_bucket: "unused".to_string(),
                    s3_region: "us-east-1".to_string(),
                    s3_addressing_style: buzz_media::S3AddressingStyle::Path,
                    max_image_bytes: 1,
                    max_gif_bytes: 1,
                    max_video_bytes: 1,
                    max_file_bytes: 1,
                    public_base_url: "http://localhost/media".to_string(),
                    upload_records_enabled: false,
                    upload_ip_header: None,
                    upload_port_header: None,
                })
                .expect("construct unused media service"),
            ),
            redis: deadpool_redis::Config::from_url("redis://127.0.0.1:1")
                .create_pool(Some(deadpool_redis::Runtime::Tokio1))
                .expect("construct unused Redis pool"),
        };
        (db, services, claim)
    }

    fn deletion_test_media_storage() -> Arc<MediaStorage> {
        let endpoint = std::env::var("BUZZ_TEST_S3_ENDPOINT")
            .or_else(|_| std::env::var("BUZZ_S3_ENDPOINT"))
            .expect("BUZZ_TEST_S3_ENDPOINT or BUZZ_S3_ENDPOINT is required");
        let access_key = std::env::var("BUZZ_TEST_S3_ACCESS_KEY")
            .or_else(|_| std::env::var("BUZZ_S3_ACCESS_KEY"))
            .expect("BUZZ_TEST_S3_ACCESS_KEY or BUZZ_S3_ACCESS_KEY is required");
        let secret_key = std::env::var("BUZZ_TEST_S3_SECRET_KEY")
            .or_else(|_| std::env::var("BUZZ_S3_SECRET_KEY"))
            .expect("BUZZ_TEST_S3_SECRET_KEY or BUZZ_S3_SECRET_KEY is required");
        let bucket = std::env::var("BUZZ_TEST_S3_BUCKET")
            .or_else(|_| std::env::var("BUZZ_S3_BUCKET"))
            .expect("BUZZ_TEST_S3_BUCKET or BUZZ_S3_BUCKET is required");
        Arc::new(
            MediaStorage::new(&buzz_media::MediaConfig {
                s3_endpoint: endpoint,
                s3_access_key: access_key,
                s3_secret_key: secret_key,
                s3_bucket: bucket,
                s3_region: std::env::var("BUZZ_TEST_S3_REGION")
                    .or_else(|_| std::env::var("BUZZ_S3_REGION"))
                    .unwrap_or_else(|_| "us-east-1".to_string()),
                s3_addressing_style: buzz_media::S3AddressingStyle::Path,
                max_image_bytes: 1,
                max_gif_bytes: 1,
                max_video_bytes: 1,
                max_file_bytes: 1,
                public_base_url: "http://localhost/media".to_string(),
                upload_records_enabled: false,
                upload_ip_header: None,
                upload_port_header: None,
            })
            .expect("construct deletion test media service"),
        )
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn approved_stage_allows_post_inventory_row_churn_before_fencing() {
        let (db, services, claim) = claimed_test_deletion("deletion-row-churn").await;
        let frozen: FrozenInventory = serde_json::from_value(
            claim
                .request
                .inventory_manifest
                .clone()
                .expect("frozen inventory"),
        )
        .expect("decode frozen inventory");

        db.add_to_allowlist(claim.request.community_id, &[0x41; 32], &[0x42; 32], None)
            .await
            .expect("post-inventory serving write");
        let live = services
            .store
            .inventory_schema(claim.request.community_id)
            .await
            .expect("live inventory after serving churn");
        assert_eq!(live.scoped_tables, frozen.schema.scoped_tables);
        assert_eq!(live.fenced_tables, frozen.schema.fenced_tables);
        assert_eq!(
            live.row_counts["pubkey_allowlist"],
            frozen.schema.row_counts["pubkey_allowlist"] + 1
        );

        execute_stage(&services, &claim, &CancellationToken::new())
            .await
            .expect("row-count churn must not fail structural revalidation");
        let fenced = services
            .store
            .get(claim.request.id)
            .await
            .expect("load fenced request");
        assert_eq!(fenced.stage, DeletionStage::Fenced);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn frozen_inventory_digest_and_storage_ownership_fail_closed() {
        let (_, _, claim) = claimed_test_deletion("deletion-integrity").await;
        assert!(validate_frozen_inventory(&claim.request).is_ok());

        let mut digest_tampered = claim.request.clone();
        digest_tampered.inventory_manifest = Some(serde_json::json!({
            "schema": {"scoped_tables": [], "row_counts": {}, "fenced_tables": []},
            "storage": {"version": 4, "prefixes": []}
        }));
        assert!(validate_frozen_inventory(&digest_tampered).is_err());

        // A manifest scoped to another community's prefixes is never the
        // deletion target's, even when internally valid.
        let foreign_manifest =
            empty_storage_manifest(buzz_core::CommunityId::from_uuid(Uuid::new_v4()));
        assert!(validate_storage_ownership(&claim.request, &foreign_manifest).is_err());
    }

    /// The non-atomic boundary under test: S3 committed a chunk's deletes,
    /// then the worker died before the chunk stamp. Resume must re-delete the
    /// chunk (missing keys report as deleted — idempotent), stamp it, and
    /// finish the stage.
    #[tokio::test]
    #[ignore = "requires Postgres and S3-compatible storage"]
    async fn drained_stage_resumes_chunk_deleted_before_stamp() {
        let (_, mut services, claim) = claimed_test_deletion("deletion-chunk-resume").await;
        services.media = deletion_test_media_storage();
        let community = claim.request.community_id;
        let meta_prefix = format!("_meta/{community}/");
        let keys = vec![
            format!("{meta_prefix}{}.json", "a".repeat(64)),
            format!("{meta_prefix}{}.json", "b".repeat(64)),
        ];
        for key in &keys {
            services
                .media
                .put(key, b"chunk-resume", "application/json")
                .await
                .expect("seed object");
        }

        services
            .store
            .begin_quiescing(&claim.lease)
            .await
            .expect("quiesce");
        let generation = services.store.fence(&claim.lease).await.expect("fence");
        let token = LeaseToken {
            fence_generation: Some(generation),
            ..claim.lease.clone()
        };
        // Two single-key chunks so resume order is observable.
        services
            .store
            .append_manifest_key_chunk(&token, 0, &meta_prefix, &keys[..1])
            .await
            .expect("append chunk 0");
        services
            .store
            .append_manifest_key_chunk(&token, 1, &meta_prefix, &keys[1..])
            .await
            .expect("append chunk 1");
        let mut digest = KeyStreamDigest::new();
        for key in &keys {
            digest.fold(key).expect("fold key");
        }
        let (keys_digest, object_count) = digest.finish();
        let mut storage = empty_storage_manifest(community);
        storage.prefixes[0] = PrefixManifest {
            prefix: meta_prefix.clone(),
            object_count,
            total_bytes: keys.len() as u64 * "chunk-resume".len() as u64,
            keys_digest,
        };
        services
            .store
            .freeze_destructive_storage_manifest(&token, &storage)
            .await
            .expect("freeze chunked manifest");
        services.store.mark_drained(&token).await.expect("drained");

        // Simulate the crash window: chunk 0's key is already gone from S3
        // but the chunk was never stamped.
        services
            .media
            .delete(&keys[0])
            .await
            .expect("simulate committed delete before stamp");

        let resumed = ClaimedDeletion {
            request: services
                .store
                .get(token.request_id)
                .await
                .expect("reload drained request"),
            lease: claim.lease,
        };
        execute_stage(&services, &resumed, &CancellationToken::new())
            .await
            .expect("Drained stage resumes at the unstamped chunk");

        assert_eq!(
            services
                .store
                .manifest_chunk_progress(token.request_id)
                .await
                .expect("chunk progress"),
            (2, 2)
        );
        assert_eq!(
            services.store.get(token.request_id).await.unwrap().stage,
            DeletionStage::BindingsRemoved
        );
        for key in &keys {
            assert!(
                !services.media.head(key).await.expect("verify absence"),
                "tenant binding {key} must be gone"
            );
        }
    }

    #[test]
    fn permanent_failures_are_typed_not_string_classified() {
        let permanent_error = permanent("catalog drift");
        let transient_error = transient("temporary catalog service reset");
        let nested = permanent_source(anyhow::anyhow!("schema mismatch")).context("outer");
        let db_permanent = anyhow::Error::from(buzz_db::DbError::DeletionSafety(
            "typed catalog drift".to_string(),
        ));
        let db_transient = anyhow::Error::from(buzz_db::DbError::Sqlx(sqlx::Error::PoolTimedOut));
        assert!(is_permanent_error(&permanent_error));
        assert!(is_permanent_error(&nested));
        assert!(is_permanent_error(&db_permanent));
        assert!(!is_permanent_error(&transient_error));
        assert!(!is_permanent_error(&db_transient));
    }

    #[test]
    fn deletion_configuration_requires_every_destructive_dependency() {
        let variable = format!("BUZZ_DELETION_REQUIRED_TEST_{}", Uuid::new_v4().simple());
        assert!(required_env(&variable).is_err());
        std::env::set_var(&variable, "   ");
        assert!(required_env(&variable).is_err());
        std::env::set_var(&variable, "configured");
        assert_eq!(
            required_env(&variable).expect("configured environment variable"),
            "configured"
        );
        std::env::remove_var(&variable);
    }

    #[test]
    fn deletion_s3_region_matches_relay_precedence_and_default() {
        assert_eq!(resolve_s3_region(None, None), "us-east-1");
        assert_eq!(
            resolve_s3_region(Some("   ".to_string()), None),
            "us-east-1"
        );
        assert_eq!(
            resolve_s3_region(Some("   ".to_string()), Some(" us-west-2 ".to_string())),
            "us-west-2"
        );
        assert_eq!(
            resolve_s3_region(None, Some("us-west-2".to_string())),
            "us-west-2"
        );
        assert_eq!(
            resolve_s3_region(
                Some("eu-central-1".to_string()),
                Some("us-west-2".to_string())
            ),
            "eu-central-1"
        );
    }

    #[test]
    fn redis_absence_requires_terminal_cursor_and_all_pages_empty() {
        assert!(!scan_proves_absence(&[(9, Vec::new())]));
        assert!(!scan_proves_absence(&[
            (9, Vec::new()),
            (0, vec!["buzz:tenant:late".to_string()]),
        ]));
        assert!(scan_proves_absence(&[(9, Vec::new()), (0, Vec::new())]));
    }

    #[tokio::test]
    #[ignore = "requires Postgres and S3-compatible storage"]
    async fn final_storage_verification_rejects_late_target_binding() {
        let (_, mut services, claim) = claimed_test_deletion("deletion-late-binding").await;
        services.media = deletion_test_media_storage();
        let community = claim.request.community_id;
        let late_key = format!("_meta/{community}/{}.json", "a".repeat(64));
        services
            .media
            .put(&late_key, b"late", "application/json")
            .await
            .expect("seed late binding");
        let error = verify_storage_absence(&services, &claim.request)
            .await
            .expect_err("late target binding must fail verification");
        assert!(format!("{error:#}").contains(&late_key));
        services
            .media
            .delete(&late_key)
            .await
            .expect("remove late binding");
        verify_storage_absence(&services, &claim.request)
            .await
            .expect("empty tenant prefixes verify clean");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn stale_lease_during_failure_recording_is_lost_ownership() {
        let (_, services, claim) = claimed_test_deletion("deletion-stale-record").await;
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .expect("test database URL");
        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("connect stale-record test DB");
        sqlx::query(
            "UPDATE community_deletion_requests SET lease_until = now() - interval '1 second' WHERE id = $1",
        )
        .bind(claim.request.id)
        .execute(&pool)
        .await
        .expect("expire claim");
        let recorded = record_stage_failure(
            &services,
            &claim.lease,
            claim.request.stage,
            &transient("test failure"),
        )
        .await
        .expect("stale ownership is not fatal");
        assert!(!recorded);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn serving_guard_heartbeats_long_operation_through_quiescing() {
        let (db, services, claim) = claimed_test_deletion("serving-guard-quiesce").await;
        let community = claim.request.community_id;
        let guard = acquire_serving_write_with_heartbeat(
            &db,
            community,
            "test_quiesce",
            Duration::from_millis(10),
        )
        .await
        .expect("serving guard");

        services
            .store
            .begin_quiescing(&claim.lease)
            .await
            .expect("quiesce");
        assert!(
            services
                .store
                .acquire_serving_write_lease(
                    community,
                    "late_external",
                    "late-owner",
                    DEFAULT_LEASE_DURATION,
                )
                .await
                .is_err(),
            "quiescing must reject newly admitted work"
        );

        let completed = Arc::new(AtomicBool::new(false));
        let operation_completed = Arc::clone(&completed);
        let result = guard
            .protect(async move {
                tokio::time::sleep(Duration::from_millis(75)).await;
                operation_completed.store(true, Ordering::Relaxed);
            })
            .await;
        assert!(
            !guard.lost().is_cancelled(),
            "quiescing must not cancel the admitted lease heartbeat"
        );
        result.expect("admitted operation survives quiescing heartbeats");
        assert!(completed.load(Ordering::Relaxed));
        assert!(matches!(
            services.store.fence(&claim.lease).await,
            Err(buzz_db::DbError::ServingWritesNotDrained {
                active_count: 1,
                ..
            })
        ));

        guard.finish().await.expect("release serving guard");
        assert_eq!(services.store.fence(&claim.lease).await.expect("fence"), 1);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn serving_guard_cancels_protected_operation_when_heartbeat_is_lost() {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .expect("BUZZ_TEST_DATABASE_URL or DATABASE_URL is required");
        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("connect serving guard test DB");
        let db = Db::from_pool(pool.clone());
        db.migrate().await.expect("migrate serving guard test DB");
        let community = db
            .ensure_configured_community(&format!(
                "serving-guard-{}.example",
                Uuid::new_v4().simple()
            ))
            .await
            .expect("create test community")
            .id;
        let guard = acquire_serving_write_with_heartbeat(
            &db,
            community,
            "test_cancel",
            Duration::from_millis(10),
        )
        .await
        .expect("serving guard");
        sqlx::query("DELETE FROM community_serving_write_leases WHERE community_id = $1")
            .bind(community.as_uuid())
            .execute(&pool)
            .await
            .expect("force heartbeat failure");
        let completed = Arc::new(AtomicBool::new(false));
        let operation_completed = Arc::clone(&completed);
        let result = guard
            .protect(async move {
                tokio::time::sleep(Duration::from_secs(1)).await;
                operation_completed.store(true, Ordering::Relaxed);
            })
            .await;
        assert!(result.is_err(), "lease loss must reject the operation");
        assert!(
            !completed.load(Ordering::Relaxed),
            "lease loss must cancel the protected operation future"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn guarded_external_step_rejects_preexisting_heartbeat_loss_without_polling_operation() {
        let (_, services, claim) = claimed_test_deletion("deletion-heartbeat").await;
        let heartbeat_lost = CancellationToken::new();
        heartbeat_lost.cancel();
        let polled = Arc::new(AtomicBool::new(false));
        let operation_polled = Arc::clone(&polled);
        let result = run_guarded_external_step(
            &services,
            &claim.lease,
            DeletionStage::Approved,
            &heartbeat_lost,
            || async move {
                operation_polled.store(true, Ordering::Relaxed);
                Ok(())
            },
        )
        .await;
        assert!(result.is_err(), "heartbeat loss must abort the side effect");
        assert!(
            result
                .expect_err("heartbeat loss error")
                .downcast_ref::<DeletionLeaseLost>()
                .is_some(),
            "heartbeat loss must stay typed"
        );
        assert!(
            !polled.load(Ordering::Relaxed),
            "a pre-cancelled heartbeat must win before polling the operation"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn shutdown_during_stage_releases_claim_without_recording_retry() {
        let (_, services, claim) = claimed_test_deletion("deletion-shutdown").await;
        let request_id = claim.request.id;
        let retry_count = claim.request.retry_count;
        let shutdown = CancellationToken::new();
        let cancel = shutdown.clone();
        let services_for_run = services.clone();
        let executor = tokio::spawn(async move {
            execute_claim(&services_for_run, LoopMode::Drain, claim, &shutdown).await
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        cancel.cancel();
        let output = tokio::time::timeout(Duration::from_secs(2), executor)
            .await
            .expect("shutdown must cancel the active stage")
            .expect("deletion executor task")
            .expect("graceful deletion executor shutdown");
        let request = services
            .store
            .get(request_id)
            .await
            .expect("load deletion request after shutdown");
        assert_eq!(output.stage, DeletionStage::Approved);
        assert_eq!(request.stage, DeletionStage::Approved);
        assert_eq!(request.retry_count, retry_count);
        assert!(request.last_error.is_none());
        assert!(request.lease_owner.is_none());
        assert!(request.lease_until.is_none());
    }

    #[tokio::test]
    async fn stage_wait_treats_shutdown_as_control_flow() {
        let shutdown = CancellationToken::new();
        shutdown.cancel();
        let heartbeat_lost = CancellationToken::new();
        let outcome = await_stage(
            std::future::pending::<Result<()>>(),
            &shutdown,
            &heartbeat_lost,
        )
        .await;
        assert!(matches!(outcome, StageOutcome::Shutdown));
    }

    #[tokio::test]
    async fn stage_wait_prioritizes_heartbeat_loss_over_a_ready_operation() {
        let shutdown = CancellationToken::new();
        let heartbeat_lost = CancellationToken::new();
        heartbeat_lost.cancel();
        let outcome = await_stage(async { Ok(()) }, &shutdown, &heartbeat_lost).await;
        match outcome {
            StageOutcome::Failed(error) => assert!(
                error.downcast_ref::<DeletionLeaseLost>().is_some(),
                "heartbeat loss must stay typed"
            ),
            StageOutcome::Completed | StageOutcome::Shutdown => {
                panic!("preexisting heartbeat loss must win")
            }
        }
    }
}
