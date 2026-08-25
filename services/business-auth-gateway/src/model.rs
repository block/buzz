use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct Principal {
    pub user_id: Uuid,
    pub issuer: String,
    pub subject: String,
    pub email: Option<String>,
    pub display_name: String,
    pub sid: Option<String>,
    pub workbench_session_id: Uuid,
    pub token_expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    pub id: Uuid,
    pub buzz_pubkey: String,
    pub device_id: String,
    pub device_name: String,
    pub device_platform: String,
    pub status: String,
    pub bound_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub version: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChallengeRequest {
    pub pubkey: String,
    pub device_id: String,
    pub device_name: String,
    pub device_platform: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChallengeResponse {
    pub id: Uuid,
    pub audience: &'static str,
    pub payload: String,
    pub expires_at: DateTime<Utc>,
    pub trace_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerifyBindingRequest {
    pub challenge_id: Uuid,
    pub signed_event: nostr::Event,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EmbedTarget {
    pub r#type: String,
    pub id: String,
    pub path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EmbedSource {
    pub buzz_channel_id: Option<String>,
    pub buzz_event_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueEmbedRequest {
    pub target: EmbedTarget,
    pub source: Option<EmbedSource>,
    pub pubkey: String,
    pub device_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueEmbedResponse {
    pub id: Uuid,
    pub embed_url: String,
    pub expires_at: DateTime<Utc>,
    pub trace_id: Uuid,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnterpriseUserSummary {
    pub id: Uuid,
    pub email: Option<String>,
    pub display_name: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeResponse {
    pub user: EnterpriseUserSummary,
    pub workbench_session_id: Uuid,
    pub bindings: Vec<Binding>,
}

#[derive(Clone, Default)]
pub struct RequestFacts {
    pub ip: Option<String>,
    pub user_agent_hash: Option<Vec<u8>>,
    pub trace_id: Uuid,
}

#[derive(Clone)]
pub struct Audit {
    pub event_type: &'static str,
    pub result: &'static str,
    pub reason: Option<&'static str>,
    pub user_id: Option<Uuid>,
    pub issuer: Option<String>,
    pub subject: Option<String>,
    pub binding_id: Option<Uuid>,
    pub pubkey_short: Option<String>,
    pub device_id: Option<String>,
    pub workbench_session_id: Option<Uuid>,
    pub embed_session_id: Option<Uuid>,
    pub business_session_id: Option<Uuid>,
    pub delegation_id: Option<Uuid>,
    pub agent_id: Option<String>,
    pub agent_turn_id: Option<String>,
    pub source_buzz_event_id: Option<String>,
    pub response_buzz_event_id: Option<String>,
    pub source_channel_id: Option<String>,
    pub tool_name: Option<String>,
    pub result_count: Option<i32>,
    pub finding_count: Option<i32>,
    pub resource_ref_count: Option<i32>,
    pub rule_set_version: Option<String>,
    pub anomaly_run_id: Option<Uuid>,
    pub duration_ms: Option<i64>,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub facts: RequestFacts,
    pub metadata: serde_json::Value,
}

impl Audit {
    pub fn event(event_type: &'static str, result: &'static str, facts: RequestFacts) -> Self {
        Self {
            event_type,
            result,
            reason: None,
            user_id: None,
            issuer: None,
            subject: None,
            binding_id: None,
            pubkey_short: None,
            device_id: None,
            workbench_session_id: None,
            embed_session_id: None,
            business_session_id: None,
            delegation_id: None,
            agent_id: None,
            agent_turn_id: None,
            source_buzz_event_id: None,
            response_buzz_event_id: None,
            source_channel_id: None,
            tool_name: None,
            result_count: None,
            finding_count: None,
            resource_ref_count: None,
            rule_set_version: None,
            anomaly_run_id: None,
            duration_ms: None,
            target_type: None,
            target_id: None,
            facts,
            metadata: serde_json::json!({}),
        }
    }
}
