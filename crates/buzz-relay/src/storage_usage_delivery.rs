//! Push each new per-community storage snapshot to an operator endpoint.
//!
//! `buzz-admin storage-snapshot` replaces one durable snapshot row per run.
//! When `BUZZ_STORAGE_USAGE_URL` is set, one relay pod (a session advisory
//! lock) checks that row every few minutes and, when its `completed_at` is
//! new, POSTs the per-community totals in batches, NIP-98 signed with the
//! relay key exactly as operator-listener mention notifications are.
//!
//! The snapshot row is the retry record: it stays until the next worker run
//! replaces it, and an undelivered snapshot is re-sent on every check. Which
//! snapshot was delivered is remembered only in memory, so a restart or
//! leader change re-sends the latest one. Every record carries an idempotency
//! key derived from the community and the snapshot time, so receivers must
//! treat a repeated key as already accepted.

use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use serde::Serialize;
use tracing::{error, info, warn};
use uuid::Uuid;

use buzz_db::storage_accounting::{StorageUsageCommunity, StoredStorageSnapshot};
use buzz_media::BucketSnapshot;

use crate::{nip98::nip98_header, state::AppState};

/// Deployment-global advisory lock electing the single delivering pod.
const STORAGE_USAGE_DELIVERY_LOCK_KEY: i64 = 0x4255_5A5A_5553_4744;
const CHECK_INTERVAL: Duration = Duration::from_secs(5 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Communities per POST, bounding each request body.
const BATCH_SIZE: usize = 500;

/// Parse `BUZZ_STORAGE_USAGE_URL`: an HTTP(S) URL without credentials, query,
/// or fragment, the same shape operator-listener endpoints must have.
pub fn parse_delivery_url(raw: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(raw.trim())
        .map_err(|e| format!("BUZZ_STORAGE_USAGE_URL is not a valid URL: {e}"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "BUZZ_STORAGE_USAGE_URL must be an HTTP(S) URL without credentials, query, or fragment"
                .to_string(),
        );
    }
    Ok(url)
}

/// One POST body. `batch` is zero-based; a snapshot is complete once every
/// batch in `0..batches` has been accepted.
#[derive(Debug, Serialize)]
struct StorageUsageBatch<'a> {
    v: u8,
    snapshot_completed_at: i64,
    batch: usize,
    batches: usize,
    communities: &'a [CommunityUsage],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CommunityUsage {
    idempotency_key: String,
    community_id: Uuid,
    community_host: Option<String>,
    storage_bytes: u64,
    storage_objects: u64,
}

/// Every snapshot community with its bytes, plus an explicit zero for each
/// active community the snapshot does not mention, so a receiver keeping the
/// latest value per community never keeps a stale nonzero one. Sorted by id
/// so batch boundaries are stable across re-sends.
fn usage_records(
    snapshot: &BucketSnapshot,
    completed_at: i64,
    communities: &[StorageUsageCommunity],
) -> Vec<CommunityUsage> {
    let hosts: HashMap<Uuid, &str> = communities
        .iter()
        .map(|community| (community.id, community.host.as_str()))
        .collect();
    let record = |community_id: Uuid, storage_bytes: u64, storage_objects: u64| CommunityUsage {
        idempotency_key: format!("{community_id}:{completed_at}"),
        community_id,
        community_host: hosts.get(&community_id).map(|host| (*host).to_string()),
        storage_bytes,
        storage_objects,
    };
    let mut records: Vec<CommunityUsage> = snapshot
        .per_community
        .iter()
        .map(|(id, storage)| record(*id, storage.bytes, storage.objects))
        .collect();
    records.extend(
        communities
            .iter()
            .filter(|community| {
                community.active && !snapshot.per_community.contains_key(&community.id)
            })
            .map(|community| record(community.id, 0, 0)),
    );
    records.sort_by_key(|record| record.community_id);
    records
}

#[async_trait::async_trait]
trait UsageSource: Sync {
    async fn completed_at(&self) -> Result<Option<DateTime<Utc>>, String>;
    async fn snapshot(&self) -> Result<Option<StoredStorageSnapshot>, String>;
    async fn communities(&self) -> Result<Vec<StorageUsageCommunity>, String>;
}

struct DbUsageSource<'a> {
    db: &'a buzz_db::Db,
}

#[async_trait::async_trait]
impl UsageSource for DbUsageSource<'_> {
    async fn completed_at(&self) -> Result<Option<DateTime<Utc>>, String> {
        self.db
            .load_storage_accounting_completed_at()
            .await
            .map_err(|error| error.to_string())
    }

    async fn snapshot(&self) -> Result<Option<StoredStorageSnapshot>, String> {
        self.db
            .load_storage_accounting_snapshot()
            .await
            .map_err(|error| error.to_string())
    }

    async fn communities(&self) -> Result<Vec<StorageUsageCommunity>, String> {
        self.db
            .storage_usage_communities()
            .await
            .map_err(|error| error.to_string())
    }
}

#[async_trait::async_trait]
trait DeliveryTransport: Sync {
    async fn post(
        &self,
        url: &url::Url,
        authorization: &str,
        body: Vec<u8>,
    ) -> Result<StatusCode, String>;
}

struct ReqwestDeliveryTransport {
    http: reqwest::Client,
}

#[async_trait::async_trait]
impl DeliveryTransport for ReqwestDeliveryTransport {
    async fn post(
        &self,
        url: &url::Url,
        authorization: &str,
        body: Vec<u8>,
    ) -> Result<StatusCode, String> {
        self.http
            .post(url.clone())
            .header("Authorization", authorization)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map(|response| response.status())
            .map_err(|error| error.to_string())
    }
}

fn build_delivery_http_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
}

/// Result of one check against the stored snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckOutcome {
    /// No snapshot exists, or the stored one was already handled.
    Unchanged,
    /// Every batch was accepted.
    Delivered,
    /// The endpoint refused a batch with a non-retryable status; this
    /// snapshot is abandoned and the next worker run is delivered normally.
    Rejected,
    /// A retryable failure; the same snapshot is re-sent on the next check.
    Retry,
}

impl CheckOutcome {
    fn metric_label(self) -> Option<&'static str> {
        match self {
            Self::Unchanged => None,
            Self::Delivered => Some("accepted"),
            Self::Rejected => Some("rejected"),
            Self::Retry => Some("retry"),
        }
    }
}

/// Deliver the stored snapshot if it differs from `handled`, the
/// `completed_at` of the last snapshot this pod delivered or abandoned.
/// Updates `handled` only on a terminal outcome.
async fn check_once<S: UsageSource, T: DeliveryTransport>(
    source: &S,
    transport: &T,
    url: &url::Url,
    relay_keypair: &nostr::Keys,
    handled: &mut Option<DateTime<Utc>>,
) -> CheckOutcome {
    let latest = match source.completed_at().await {
        Ok(Some(latest)) => latest,
        Ok(None) => return CheckOutcome::Unchanged,
        Err(error) => {
            warn!(%error, "storage-usage delivery could not read the snapshot time");
            return CheckOutcome::Retry;
        }
    };
    if *handled == Some(latest) {
        return CheckOutcome::Unchanged;
    }
    let stored = match source.snapshot().await {
        Ok(Some(stored)) => stored,
        Ok(None) => return CheckOutcome::Unchanged,
        Err(error) => {
            warn!(%error, "storage-usage delivery could not load the snapshot");
            return CheckOutcome::Retry;
        }
    };
    let snapshot: BucketSnapshot = match serde_json::from_value(stored.snapshot) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            // Re-reading the same row cannot succeed; wait for the next run.
            error!(%error, "stored storage snapshot does not decode; skipping it");
            *handled = Some(stored.completed_at);
            return CheckOutcome::Rejected;
        }
    };
    let communities = match source.communities().await {
        Ok(communities) => communities,
        Err(error) => {
            warn!(%error, "storage-usage delivery could not list communities");
            return CheckOutcome::Retry;
        }
    };

    let completed_at = stored.completed_at.timestamp();
    let records = usage_records(&snapshot, completed_at, &communities);
    let batches: Vec<&[CommunityUsage]> = records.chunks(BATCH_SIZE).collect();
    for (index, communities) in batches.iter().enumerate() {
        let body = match serde_json::to_vec(&StorageUsageBatch {
            v: 1,
            snapshot_completed_at: completed_at,
            batch: index,
            batches: batches.len(),
            communities,
        }) {
            Ok(body) => body,
            Err(error) => {
                error!(%error, "storage-usage batch encoding failed");
                *handled = Some(stored.completed_at);
                return CheckOutcome::Rejected;
            }
        };
        let auth = match nip98_header(relay_keypair, url.as_str(), &body) {
            Ok(auth) => auth,
            Err(error) => {
                error!(%error, "storage-usage batch signing failed");
                return CheckOutcome::Retry;
            }
        };
        match transport.post(url, &auth, body).await {
            Ok(status) if status.is_success() => {}
            Ok(status)
                if status == StatusCode::REQUEST_TIMEOUT
                    || status == StatusCode::TOO_MANY_REQUESTS
                    || status.is_server_error() =>
            {
                warn!(%status, batch = index, "storage-usage delivery failed; retrying next check");
                return CheckOutcome::Retry;
            }
            Ok(status) => {
                error!(%status, batch = index, "storage-usage delivery rejected; skipping this snapshot");
                *handled = Some(stored.completed_at);
                return CheckOutcome::Rejected;
            }
            Err(error) => {
                warn!(%error, batch = index, "storage-usage delivery failed; retrying next check");
                return CheckOutcome::Retry;
            }
        }
    }
    *handled = Some(stored.completed_at);
    metrics::gauge!("buzz_storage_usage_delivered_snapshot_timestamp_seconds")
        .set(completed_at as f64);
    info!(
        snapshot_completed_at = completed_at,
        communities = records.len(),
        "storage-usage snapshot delivered"
    );
    CheckOutcome::Delivered
}

/// Check for a new snapshot every few minutes while holding the delivery lock.
pub async fn run_delivery_worker(state: Arc<AppState>, url: url::Url) {
    let transport = match build_delivery_http_client() {
        Ok(http) => ReqwestDeliveryTransport { http },
        Err(error) => {
            error!(%error, "storage-usage HTTP client initialization failed");
            return;
        }
    };
    let source = DbUsageSource { db: &state.db };
    let mut leader: Option<buzz_db::SessionLeader> = None;
    let mut handled = None;
    let mut interval = tokio::time::interval(CHECK_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        if let Some(guard) = leader.as_mut() {
            if !guard.is_live().await {
                warn!("storage-usage delivery lock session lost; re-electing");
                leader = None;
            }
        }
        if leader.is_none() {
            match state
                .db
                .try_lock_session_leader(STORAGE_USAGE_DELIVERY_LOCK_KEY)
                .await
            {
                Ok(acquired) => leader = acquired,
                Err(error) => {
                    warn!(%error, "storage-usage delivery lock attempt failed");
                    continue;
                }
            }
        }
        if leader.is_none() {
            continue;
        }
        let outcome = check_once(
            &source,
            &transport,
            &url,
            &state.relay_keypair,
            &mut handled,
        )
        .await;
        if let Some(label) = outcome.metric_label() {
            metrics::counter!("buzz_storage_usage_deliveries_total", "outcome" => label)
                .increment(1);
        }
    }
}

#[cfg(test)]
mod tests;
