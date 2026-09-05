use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const STORE_VERSION: u32 = 1;
pub const SCHEDULE_COPY: &str =
    "Daily at 09:00 Asia/Manila; liveness every 30 seconds while Buzz Desktop is running";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OperationsConfig {
    pub enabled: bool,
    pub channel_id: Option<String>,
    pub assistant_pubkey: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DigestDelivery {
    pub date: String,
    pub marker: String,
    pub event_id: Option<String>,
    pub event_created_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmedDigest {
    pub date: String,
    pub event_id: String,
    pub event_created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OutageEpisode {
    pub id: String,
    pub agent_pubkey: String,
    pub first_detected_at: i64,
    pub active: bool,
    pub classification: String,
    pub last_exit_code: Option<i32>,
    pub last_error_code: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AlertBatch {
    pub marker: String,
    pub episode_ids: Vec<String>,
    pub event_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScopeDeliveryState {
    pub confirmed_digest: Option<ConfirmedDigest>,
    pub digest_wakes: Vec<DigestDelivery>,
    pub metric_coverage_since: Option<i64>,
    pub episodes: BTreeMap<String, OutageEpisode>,
    pub alert_batches: Vec<AlertBatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScopedOperations {
    pub owner_pubkey: String,
    pub relay_url: String,
    pub config: OperationsConfig,
    #[serde(default)]
    pub delivery: ScopeDeliveryState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OperationsStore {
    pub version: u32,
    #[serde(default)]
    pub scopes: Vec<ScopedOperations>,
}

impl Default for OperationsStore {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            scopes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationsStatus {
    pub config: OperationsConfig,
    pub schedule: &'static str,
    pub next_manila_boundary_utc: String,
    pub metric_coverage_since: Option<i64>,
    pub last_confirmed_digest: Option<ConfirmedDigest>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveOperationsConfig {
    pub enabled: bool,
    pub channel_id: Option<String>,
    pub assistant_pubkey: Option<String>,
}
