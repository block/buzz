//! Push each new per-community storage snapshot to an operator endpoint.
//!
//! `buzz-admin storage-snapshot` replaces one durable snapshot row per run.
//! When `BUZZ_STORAGE_USAGE_URL` is set, one relay pod (a session advisory
//! lock) checks that row every few minutes and, when its `completed_at` is
//! new, POSTs the per-community totals in batches, in the request shape of
//! kgoose's `POST /v1/buzz/storage-usage/record`. Each batch is NIP-98 signed
//! with the relay key, as operator-listener mention notifications are.
//!
//! The snapshot row is the retry record: it stays until the next worker run
//! replaces it, and an undelivered snapshot is re-sent on every check. Which
//! snapshot was delivered is remembered only in memory, so a restart or
//! leader change re-sends the latest one. Receivers must treat a repeated
//! community and `measured_at` as already recorded.

use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::{DateTime, SecondsFormat, Utc};
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
/// Communities per POST. One run sends every batch in order, so this bounds
/// each request body, not how many communities a snapshot reports.
const BATCH_SIZE: usize = 50;

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

/// One POST body: kgoose's `RecordBuzzStorageUsageRequest`. Batches are
/// independent requests sharing `measured_at`, the snapshot's RFC 3339
/// completion time.
#[derive(Debug, Serialize)]
struct RecordStorageUsageRequest<'a> {
    measured_at: &'a str,
    communities: &'a [CommunityUsage],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CommunityUsage {
    community_host: String,
    storage_bytes: u64,
    storage_objects: u64,
}

/// Every snapshot community with its bytes, plus an explicit zero for each
/// active community the snapshot does not mention, so a receiver keeping the
/// latest value per community never keeps a stale nonzero one. A snapshot
/// community with no community row has no host to report and is skipped.
/// Hosts are unique, so sorting by host keeps batch boundaries stable across
/// re-sends.
fn usage_records(
    snapshot: &BucketSnapshot,
    communities: &[StorageUsageCommunity],
) -> Vec<CommunityUsage> {
    let hosts: HashMap<Uuid, &str> = communities
        .iter()
        .map(|community| (community.id, community.host.as_str()))
        .collect();
    let mut unmatched = 0usize;
    let mut records: Vec<CommunityUsage> = snapshot
        .per_community
        .iter()
        .filter_map(|(id, storage)| {
            let Some(host) = hosts.get(id) else {
                unmatched += 1;
                return None;
            };
            Some(CommunityUsage {
                community_host: (*host).to_string(),
                storage_bytes: storage.bytes,
                storage_objects: storage.objects,
            })
        })
        .collect();
    records.extend(
        communities
            .iter()
            .filter(|community| {
                community.active && !snapshot.per_community.contains_key(&community.id)
            })
            .map(|community| CommunityUsage {
                community_host: community.host.clone(),
                storage_bytes: 0,
                storage_objects: 0,
            }),
    );
    if unmatched > 0 {
        warn!(
            unmatched,
            "storage snapshot communities without a community row were not reported"
        );
    }
    records.sort_by(|a, b| a.community_host.cmp(&b.community_host));
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

/// What this pod has handled, kept across checks while it leads.
#[derive(Debug, Default)]
struct DeliveryProgress {
    /// `completed_at` of the last snapshot delivered or abandoned. Set only
    /// on a terminal outcome.
    handled: Option<DateTime<Utc>>,
    /// `completed_at` of the last snapshot whose every batch was accepted.
    delivered: Option<DateTime<Utc>>,
}

/// One leader check: deliver the stored snapshot if it is new, then record
/// the outcome. The delivered-snapshot gauge is re-published on every check,
/// including unchanged and failed ones, so it outlives the exporter's gauge
/// idle timeout between daily snapshots and a freshness alert always has a
/// sample to fire on.
async fn check_once<S: UsageSource, T: DeliveryTransport>(
    source: &S,
    transport: &T,
    url: &url::Url,
    relay_keypair: &nostr::Keys,
    progress: &mut DeliveryProgress,
) -> CheckOutcome {
    let outcome = deliver_new_snapshot(source, transport, url, relay_keypair, progress).await;
    if let Some(label) = outcome.metric_label() {
        metrics::counter!("buzz_storage_usage_deliveries_total", "outcome" => label).increment(1);
    }
    if let Some(delivered) = progress.delivered {
        metrics::gauge!("buzz_storage_usage_delivered_snapshot_timestamp_seconds")
            .set(delivered.timestamp() as f64);
    }
    outcome
}

/// Deliver the stored snapshot if it differs from `progress.handled`.
async fn deliver_new_snapshot<S: UsageSource, T: DeliveryTransport>(
    source: &S,
    transport: &T,
    url: &url::Url,
    relay_keypair: &nostr::Keys,
    progress: &mut DeliveryProgress,
) -> CheckOutcome {
    let latest = match source.completed_at().await {
        Ok(Some(latest)) => latest,
        Ok(None) => return CheckOutcome::Unchanged,
        Err(error) => {
            warn!(%error, "storage-usage delivery could not read the snapshot time");
            return CheckOutcome::Retry;
        }
    };
    if progress.handled == Some(latest) {
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
            progress.handled = Some(stored.completed_at);
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

    let measured_at = stored
        .completed_at
        .to_rfc3339_opts(SecondsFormat::Secs, true);
    let records = usage_records(&snapshot, &communities);
    for (index, communities) in records.chunks(BATCH_SIZE).enumerate() {
        let body = match serde_json::to_vec(&RecordStorageUsageRequest {
            measured_at: &measured_at,
            communities,
        }) {
            Ok(body) => body,
            Err(error) => {
                error!(%error, "storage-usage batch encoding failed");
                progress.handled = Some(stored.completed_at);
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
                progress.handled = Some(stored.completed_at);
                return CheckOutcome::Rejected;
            }
            Err(error) => {
                warn!(%error, batch = index, "storage-usage delivery failed; retrying next check");
                return CheckOutcome::Retry;
            }
        }
    }
    progress.handled = Some(stored.completed_at);
    progress.delivered = Some(stored.completed_at);
    info!(
        %measured_at,
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
    let mut progress = DeliveryProgress::default();
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
        check_once(
            &source,
            &transport,
            &url,
            &state.relay_keypair,
            &mut progress,
        )
        .await;
    }
}

#[cfg(test)]
mod tests;
