//! Dry-run inventory for future physical Git object-store garbage collection.
//!
//! This module deliberately does not delete objects. It computes a
//! deployment-global reachability snapshot and reports immutable objects that
//! were not referenced by any current repository pointer. Deletion needs a
//! durable, continuous-unreachability grace period plus coordination with
//! concurrent publishers; neither safety condition should be approximated.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::manifest::Manifest;
use super::store::{GitStore, ObjectList, StoreError, StoredObject};
use crate::state::AppState;

const MAX_GC_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
const IMMUTABLE_PREFIXES: [&str; 3] = ["manifests/", "packs/", "idx/"];
const GIT_GC_LOCK_KEY: i64 = 0x4255_5A5A_4749_5447;

/// Configuration for the singleton dry-run inventory worker.
#[derive(Debug, Clone, Copy)]
pub struct GitGcWorkerConfig {
    /// Whether the worker is enabled.
    pub enabled: bool,
    /// Delay between inventory scans.
    pub interval: Duration,
    /// Per-scan inventory limits.
    pub limits: GitGcScanLimits,
}

impl GitGcWorkerConfig {
    /// Read worker configuration from `BUZZ_GIT_GC_*` environment variables.
    pub fn from_env() -> Result<Self, String> {
        Ok(Self {
            enabled: parse_bool_env("BUZZ_GIT_GC_ENABLED", false)?,
            interval: Duration::from_secs(parse_positive_u64_env(
                "BUZZ_GIT_GC_INTERVAL_SECS",
                3_600,
            )?),
            limits: GitGcScanLimits {
                max_pointers: parse_positive_usize_env("BUZZ_GIT_GC_MAX_POINTERS", 10_000)?,
                max_objects_per_prefix: parse_positive_usize_env(
                    "BUZZ_GIT_GC_MAX_OBJECTS_PER_PREFIX",
                    10_000,
                )?,
                max_manifest_bytes: parse_positive_u64_env(
                    "BUZZ_GIT_GC_MAX_MANIFEST_BYTES_PER_SCAN",
                    512 * 1024 * 1024,
                )?,
                timeout: Duration::from_secs(parse_positive_u64_env(
                    "BUZZ_GIT_GC_SCAN_TIMEOUT_SECS",
                    300,
                )?),
            },
        })
    }
}

/// Limits for one dry-run inventory scan.
#[derive(Debug, Clone, Copy)]
pub struct GitGcScanLimits {
    /// Maximum repository pointers that may be considered.
    pub max_pointers: usize,
    /// Maximum objects listed from each immutable Git prefix.
    pub max_objects_per_prefix: usize,
    /// Maximum manifest bytes downloaded while marking live pointers.
    pub max_manifest_bytes: u64,
    /// Hard deadline for one complete inventory attempt.
    pub timeout: Duration,
}

impl Default for GitGcScanLimits {
    fn default() -> Self {
        Self {
            max_pointers: 10_000,
            max_objects_per_prefix: 10_000,
            max_manifest_bytes: 512 * 1024 * 1024,
            timeout: Duration::from_secs(300),
        }
    }
}

/// Run the leader-elected dry-run inventory worker until process shutdown.
pub async fn run_git_gc_worker(state: Arc<AppState>, config: GitGcWorkerConfig) {
    if !config.enabled {
        return;
    }

    let jitter_bound = config.interval.as_secs().max(1);
    tokio::time::sleep(Duration::from_secs(rand::random::<u64>() % jitter_bound)).await;
    let mut interval = tokio::time::interval(config.interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut leader: Option<buzz_db::UsageMetricsLeader> = None;

    loop {
        interval.tick().await;
        let mut demoted = false;
        if let Some(leader_guard) = leader.as_mut() {
            if !leader_guard.is_live().await {
                tracing::warn!("Git object-store GC inventory leader demoting");
                leader = None;
                demoted = true;
            }
        }
        if leader.is_none() && !demoted {
            // `try_lock_usage_metrics` is a generic detached-session advisory
            // lock keyed by `lock_key`; despite the name it is safe to reuse
            // with a distinct key. Renaming it is deferred to keep this change
            // out of `buzz-db`.
            match state.db.try_lock_usage_metrics(GIT_GC_LOCK_KEY).await {
                Ok(acquired) => {
                    leader = acquired;
                    if leader.is_some() {
                        tracing::info!("Acquired Git object-store GC inventory leader lock");
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "Git object-store GC leader election failed");
                }
            }
        }
        metrics::gauge!("buzz_git_object_store_gc_is_leader").set(if leader.is_some() {
            1.0
        } else {
            0.0
        });

        if leader.is_none() {
            continue;
        }
        match scan_git_object_store(&state.git_store, config.limits).await {
            Ok(report) => {
                tracing::info!(
                    pointers = report.pointers,
                    reachable_objects = report.reachable_objects,
                    observed_candidate_objects = report.observed_candidate_objects,
                    observed_candidate_bytes = report.observed_candidate_bytes,
                    pagination_complete = report.pagination_complete,
                    "Git object-store GC dry-run inventory completed"
                );
            }
            Err(error) => {
                tracing::warn!(%error, "Git object-store GC dry-run inventory failed");
            }
        }
    }
}

/// Result of one dry-run inventory scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitGcScanReport {
    /// Number of current repository pointers inspected.
    pub pointers: usize,
    /// Number of immutable Git objects marked reachable.
    pub reachable_objects: usize,
    /// Number of listed immutable objects not in the reachability set.
    pub observed_candidate_objects: usize,
    /// Total bytes represented by the candidates.
    pub observed_candidate_bytes: u64,
    /// False when any bounded prefix listing was truncated.
    pub pagination_complete: bool,
}

/// Errors from a dry-run inventory scan.
#[derive(Debug, thiserror::Error)]
pub enum GitGcError {
    /// Object-store operation failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// A live pointer or manifest was malformed, so classification failed closed.
    #[error("invalid live Git object {key}: {reason}")]
    InvalidLiveObject {
        /// Pointer or manifest key.
        key: String,
        /// Validation failure.
        reason: String,
    },
    /// Pointer inventory exceeded its safety bound.
    #[error("repository pointer inventory exceeded the configured limit")]
    PointerInventoryTruncated,
    /// Live-manifest reads exceeded the configured transfer budget.
    #[error("live manifest inventory exceeded the configured byte budget")]
    ManifestBudgetExceeded,
    /// The scan did not finish before its configured deadline.
    #[error("Git object-store inventory scan timed out")]
    TimedOut,
}

/// Scan all current Git pointers and classify unreferenced immutable objects.
///
/// This is observability only. A candidate is not necessarily safe to delete:
/// the scan does not prove continuous unreachability or exclude a concurrent
/// publisher from making an existing content-addressed object reachable.
pub async fn scan_git_object_store(
    store: &GitStore,
    limits: GitGcScanLimits,
) -> Result<GitGcScanReport, GitGcError> {
    let started = Instant::now();
    let result = tokio::time::timeout(limits.timeout, scan_git_object_store_inner(store, limits))
        .await
        .map_err(|_| GitGcError::TimedOut)
        .and_then(|result| result);
    match &result {
        Ok(report) => {
            metrics::counter!("buzz_git_object_store_gc_scans_total", "result" => "success")
                .increment(1);
            metrics::gauge!("buzz_git_object_store_gc_observed_candidate_objects")
                .set(report.observed_candidate_objects as f64);
            metrics::gauge!("buzz_git_object_store_gc_observed_candidate_bytes")
                .set(report.observed_candidate_bytes as f64);
            metrics::gauge!("buzz_git_object_store_gc_scan_complete")
                .set(if report.pagination_complete { 1.0 } else { 0.0 });
            metrics::gauge!("buzz_git_object_store_gc_last_success_timestamp_seconds")
                .set(unix_timestamp_seconds());
        }
        Err(_) => {
            metrics::counter!("buzz_git_object_store_gc_scans_total", "result" => "error")
                .increment(1);
            metrics::gauge!("buzz_git_object_store_gc_scan_complete").set(0.0);
        }
    }
    metrics::histogram!("buzz_git_object_store_gc_scan_seconds")
        .record(started.elapsed().as_secs_f64());
    result
}

async fn scan_git_object_store_inner(
    store: &GitStore,
    limits: GitGcScanLimits,
) -> Result<GitGcScanReport, GitGcError> {
    let pointer_list = store.list_prefix("repos/", limits.max_pointers).await?;
    if pointer_list.truncated {
        return Err(GitGcError::PointerInventoryTruncated);
    }

    let pointers: Vec<_> = pointer_list
        .objects
        .into_iter()
        .filter(|object| object.key.ends_with("/pointer"))
        .collect();
    let mut reachable = HashSet::new();
    let mut manifest_bytes = 0u64;
    for pointer in &pointers {
        let remaining = limits.max_manifest_bytes.saturating_sub(manifest_bytes);
        if remaining == 0 {
            return Err(GitGcError::ManifestBudgetExceeded);
        }
        manifest_bytes = manifest_bytes.saturating_add(
            mark_current_manifest(store, &pointer.key, &mut reachable, remaining).await?,
        );
    }

    let mut observed_candidate_objects = 0usize;
    let mut observed_candidate_bytes = 0u64;
    let mut pagination_complete = true;
    for prefix in IMMUTABLE_PREFIXES {
        let listed = store
            .list_prefix(prefix, limits.max_objects_per_prefix)
            .await?;
        pagination_complete &= !listed.truncated;
        let (count, bytes) = classify_candidates(&listed, &reachable);
        observed_candidate_objects = observed_candidate_objects.saturating_add(count);
        observed_candidate_bytes = observed_candidate_bytes.saturating_add(bytes);
    }

    Ok(GitGcScanReport {
        pointers: pointers.len(),
        reachable_objects: reachable.len(),
        observed_candidate_objects,
        observed_candidate_bytes,
        pagination_complete,
    })
}

async fn mark_current_manifest(
    store: &GitStore,
    pointer_key: &str,
    reachable: &mut HashSet<String>,
    remaining_manifest_bytes: u64,
) -> Result<u64, GitGcError> {
    let Some((_etag, pointer_body)) = store.get_pointer(pointer_key).await? else {
        return Ok(0);
    };
    let digest = std::str::from_utf8(&pointer_body)
        .map(str::trim)
        .map_err(|error| GitGcError::InvalidLiveObject {
            key: pointer_key.to_string(),
            reason: error.to_string(),
        })?;
    if !is_digest(digest) {
        return Err(GitGcError::InvalidLiveObject {
            key: pointer_key.to_string(),
            reason: "pointer body is not a SHA-256 digest".to_string(),
        });
    }

    let manifest_key = format!("manifests/{digest}");
    let read_limit = remaining_manifest_bytes.min(MAX_GC_MANIFEST_BYTES);
    let bytes = match store
        .get_verified_limited(&manifest_key, digest, read_limit)
        .await
    {
        Err(StoreError::ObjectTooLarge { .. })
            if remaining_manifest_bytes < MAX_GC_MANIFEST_BYTES =>
        {
            return Err(GitGcError::ManifestBudgetExceeded);
        }
        result => result?,
    };
    let manifest = Manifest::from_bytes(&bytes).map_err(|error| GitGcError::InvalidLiveObject {
        key: manifest_key.clone(),
        reason: error.to_string(),
    })?;
    manifest
        .validate()
        .map_err(|error| GitGcError::InvalidLiveObject {
            key: manifest_key.clone(),
            reason: error.to_string(),
        })?;

    reachable.insert(manifest_key);
    for pack_key in manifest.packs {
        if let Some(digest) = pack_key.strip_prefix("packs/") {
            reachable.insert(format!("idx/{digest}"));
        }
        reachable.insert(pack_key);
    }
    Ok(bytes.len() as u64)
}

fn classify_candidates(listed: &ObjectList, reachable: &HashSet<String>) -> (usize, u64) {
    listed
        .objects
        .iter()
        .filter(|object| is_gc_object(object) && !reachable.contains(&object.key))
        .fold((0usize, 0u64), |(count, bytes), object| {
            (count.saturating_add(1), bytes.saturating_add(object.size))
        })
}

fn is_gc_object(object: &StoredObject) -> bool {
    IMMUTABLE_PREFIXES
        .iter()
        .any(|prefix| object.key.strip_prefix(prefix).is_some_and(is_digest))
}

fn is_digest(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn unix_timestamp_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn parse_bool_env(name: &str, default: bool) -> Result<bool, String> {
    match std::env::var(name) {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" => Ok(true),
            "false" | "0" => Ok(false),
            _ => Err(format!("{name} must be true or false")),
        },
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} must be valid Unicode")),
    }
}

fn parse_positive_u64_env(name: &str, default: u64) -> Result<u64, String> {
    match std::env::var(name) {
        Ok(value) => value
            .parse::<u64>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| format!("{name} must be a positive integer")),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} must be valid Unicode")),
    }
}

fn parse_positive_usize_env(name: &str, default: usize) -> Result<usize, String> {
    match std::env::var(name) {
        Ok(value) => value
            .parse::<usize>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| format!("{name} must be a positive integer")),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} must be valid Unicode")),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashSet};
    use std::time::Duration;

    use super::{
        classify_candidates, is_digest, parse_bool_env, parse_positive_u64_env,
        parse_positive_usize_env, scan_git_object_store, GitGcScanLimits,
    };
    use crate::api::git::manifest::{Manifest, MANIFEST_VERSION};
    use crate::api::git::store::{GitStore, ObjectList, Precond, StoredObject};

    fn object(key: &str, size: u64) -> StoredObject {
        StoredObject {
            key: key.to_string(),
            size,
            last_modified: "2026-07-21T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn candidate_classification_is_prefix_scoped_and_reachability_aware() {
        let live_pack = format!("packs/{}", "a".repeat(64));
        let orphan_pack = format!("packs/{}", "b".repeat(64));
        let listed = ObjectList {
            objects: vec![
                object(&live_pack, 10),
                object(&orphan_pack, 20),
                object("packs/not-a-digest", 30),
                object(&format!("media/{}", "c".repeat(64)), 40),
            ],
            truncated: false,
        };
        let reachable = HashSet::from([live_pack]);

        assert_eq!(classify_candidates(&listed, &reachable), (1, 20));
    }

    #[test]
    fn digest_validation_is_exact() {
        assert!(is_digest(&"a".repeat(64)));
        assert!(!is_digest(&"a".repeat(63)));
        assert!(!is_digest(&"g".repeat(64)));
    }

    #[test]
    fn gc_environment_values_are_strict() {
        assert_eq!(
            parse_bool_env("BUZZ_TEST_GC_MISSING_BOOL", false),
            Ok(false)
        );
        assert_eq!(
            parse_positive_u64_env("BUZZ_TEST_GC_MISSING_U64", 10),
            Ok(10)
        );
        assert_eq!(
            parse_positive_usize_env("BUZZ_TEST_GC_MISSING_USIZE", 10),
            Ok(10)
        );
    }

    fn live_store() -> GitStore {
        let endpoint = std::env::var("BUZZ_GIT_S3_ENDPOINT")
            .or_else(|_| std::env::var("BUZZ_S3_ENDPOINT"))
            .unwrap_or_else(|_| "http://localhost:9000".into());
        let access_key = std::env::var("BUZZ_GIT_S3_ACCESS_KEY")
            .or_else(|_| std::env::var("BUZZ_S3_ACCESS_KEY"))
            .unwrap_or_else(|_| "buzz_dev".into());
        let secret_key = std::env::var("BUZZ_GIT_S3_SECRET_KEY")
            .or_else(|_| std::env::var("BUZZ_S3_SECRET_KEY"))
            .unwrap_or_else(|_| "buzz_dev_secret".into());
        let bucket = std::env::var("BUZZ_GIT_S3_BUCKET")
            .or_else(|_| std::env::var("BUZZ_S3_BUCKET"))
            .unwrap_or_else(|_| "buzz-media".into());
        let region = std::env::var("BUZZ_GIT_S3_REGION")
            .or_else(|_| std::env::var("BUZZ_S3_REGION"))
            .unwrap_or_else(|_| "us-east-1".into());
        GitStore::new(&endpoint, &access_key, &secret_key, &bucket, &region)
            .expect("connect to live object store")
    }

    #[tokio::test]
    async fn live_scan_marks_current_objects_and_observes_orphans() {
        if std::env::var("BUZZ_GIT_S3_PROBE").as_deref() != Ok("1") {
            return;
        }

        let store = live_store();
        let live_pack_key = store
            .put_pack(b"live-gc-pack")
            .await
            .expect("put live pack");
        let orphan_pack_key = store
            .put_pack(b"orphan-gc-pack")
            .await
            .expect("put orphan pack");
        let commit = "1".repeat(40);
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            head: "refs/heads/main".to_string(),
            refs: BTreeMap::from([("refs/heads/main".to_string(), commit)]),
            packs: vec![live_pack_key.clone()],
            parent: None,
        };
        let manifest_key = store
            .put_manifest(&manifest.canonical_bytes().expect("manifest bytes"))
            .await
            .expect("put manifest");
        let digest = manifest_key
            .strip_prefix("manifests/")
            .expect("manifest digest");
        let pointer_key = format!("repos/{}/gc-test/repo/pointer", uuid::Uuid::new_v4());
        store
            .put_pointer(&pointer_key, digest.as_bytes(), Precond::IfNoneMatchStar)
            .await
            .expect("put pointer");

        let report = scan_git_object_store(
            &store,
            GitGcScanLimits {
                max_pointers: 10_000,
                max_objects_per_prefix: 10_000,
                max_manifest_bytes: 64 * 1024 * 1024,
                timeout: Duration::from_secs(30),
            },
        )
        .await
        .expect("scan object store");
        assert!(report.pointers >= 1);
        assert!(report.reachable_objects >= 3);
        assert!(report.observed_candidate_objects >= 1);
        assert!(report.observed_candidate_bytes >= b"orphan-gc-pack".len() as u64);

        store.delete_for_test(&pointer_key).await;
        store.delete_for_test(&manifest_key).await;
        store.delete_for_test(&live_pack_key).await;
        store.delete_for_test(&orphan_pack_key).await;
    }
}
