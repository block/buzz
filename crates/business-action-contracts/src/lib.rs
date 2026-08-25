#![forbid(unsafe_code)]

use business_anomaly_contracts::{Confidence, FindingRule, Severity};
use business_query_contracts::{Money, Pagination, ResourceRef};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

pub const CATALOG_VERSION: &str = "trade-action-v1.0";
pub const BUSINESS_ACTION_READ: &str = "business_action:read";
pub const FINDING_READ: &str = "business_finding:read";
pub const FINDING_ACKNOWLEDGE: &str = "business_finding:acknowledge";
pub const ACTION_PROPOSAL_READ: &str = "business_action_proposal:read";
pub const WORK_ITEM_CREATE: &str = "business_work_item:create";
pub const WORK_ITEM_UPDATE: &str = "business_work_item:update";
pub const WORK_ITEM_ASSIGN: &str = "business_work_item:assign";
pub const WORK_ITEM_COMPLETE: &str = "business_work_item:complete";
pub const APPROVAL_DRAFT_CREATE: &str = "business_approval_draft:create";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConditionStatus {
    Active,
    Cleared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Unreviewed,
    Acknowledged,
    InProgress,
    Resolved,
    Dismissed,
    Reopened,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Suggested,
    Accepted,
    Dismissed,
    Expired,
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemDraftStatus {
    Prepared,
    Consumed,
    Expired,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemStatus {
    Open,
    InProgress,
    Blocked,
    ReadyForReview,
    Completed,
    Cancelled,
    Reopened,
}

impl WorkItemStatus {
    pub fn is_active(self) -> bool {
        !matches!(self, Self::Completed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDraftStatus {
    Draft,
    ReadyForReview,
    Withdrawn,
    Expired,
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Normal,
    High,
    Urgent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Completed,
    Partial,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DismissReasonCode {
    AcceptedBusinessRisk,
    FalsePositive,
    KnownTimingDifference,
    DuplicateProcess,
    InsufficientMateriality,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionCode {
    DataCorrected,
    BusinessReviewCompleted,
    CustomerPlanConfirmed,
    PurchasePlanReviewed,
    InventoryPlanReviewed,
    Other,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FindingScope {
    pub legal_entity_id: String,
    pub warehouse_id: Option<String>,
    pub customer_id: Option<String>,
    pub supplier_id: Option<String>,
    pub brand_id: Option<String>,
    pub business_unit_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FindingLifecycle {
    pub id: Uuid,
    pub finding_key: String,
    pub anomaly_type: String,
    pub severity: Severity,
    pub confidence: Confidence,
    pub title: String,
    pub summary_code: String,
    pub primary_resource: ResourceRef,
    pub related_resources: Vec<ResourceRef>,
    pub impact: Option<Money>,
    pub rule: FindingRule,
    pub evidence_summary: Vec<String>,
    pub data_as_of: DateTime<Utc>,
    pub scope: FindingScope,
    pub scope_hash: String,
    pub rule_set_version: String,
    pub condition_status: ConditionStatus,
    pub review_status: ReviewStatus,
    pub occurrence_count: u64,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub cleared_at: Option<DateTime<Utc>>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub dismissed_at: Option<DateTime<Utc>>,
    pub review_after: Option<DateTime<Utc>>,
    pub finding_snapshot_hash: String,
    pub version: u64,
    pub trace_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionCatalogEntry {
    pub id: Uuid,
    pub version: String,
    pub action_code: String,
    pub title: String,
    pub description: String,
    pub supported_anomaly_types: Vec<String>,
    pub target_resource_types: Vec<String>,
    pub default_due_days: u16,
    pub allowed_assignee_role_keys: Vec<String>,
    pub approval_draft_type: Option<String>,
    pub requires_explicit_confirmation: bool,
    pub enabled: bool,
    pub effective_from: DateTime<Utc>,
    pub effective_to: Option<DateTime<Utc>>,
    pub config_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionProposal {
    pub id: Uuid,
    pub proposal_number: String,
    pub finding_id: Uuid,
    pub action_catalog_version: String,
    pub action_code: String,
    pub title: String,
    pub summary: String,
    pub recommended_role_key: Option<String>,
    pub default_due_at: DateTime<Utc>,
    pub status: ProposalStatus,
    pub finding_version: u64,
    pub finding_snapshot_hash: String,
    pub rule_set_version: String,
    pub proposal_hash: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub dismissed_at: Option<DateTime<Utc>>,
    pub created_by_type: String,
    pub created_by_id: String,
    pub trace_id: Uuid,
    pub version: u64,
    pub resource_ref: ResourceRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkItemDraft {
    pub id: Uuid,
    pub proposal_id: Uuid,
    pub finding_id: Uuid,
    pub title: String,
    pub description: String,
    pub assignee_user_id: Option<Uuid>,
    pub assignee_role_key: Option<String>,
    pub due_at: DateTime<Utc>,
    pub priority: Priority,
    pub finding_snapshot_hash: String,
    pub expected_finding_version: u64,
    pub preview_hash: String,
    pub created_by_user_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub status: WorkItemDraftStatus,
    pub trace_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkItem {
    pub id: Uuid,
    pub work_item_number: String,
    pub finding_id: Uuid,
    pub proposal_id: Uuid,
    pub action_code: String,
    pub title: String,
    pub description: String,
    pub priority: Priority,
    pub status: WorkItemStatus,
    pub assignee_user_id: Option<Uuid>,
    pub assignee_role_key: Option<String>,
    pub created_by_user_id: Uuid,
    pub due_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub source_condition_status: ConditionStatus,
    pub finding_snapshot_hash: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub version: u64,
    pub trace_id: Uuid,
    pub resource_ref: ResourceRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkItemEvent {
    pub id: Uuid,
    pub work_item_id: Uuid,
    pub event_type: String,
    pub from_status: Option<WorkItemStatus>,
    pub to_status: Option<WorkItemStatus>,
    pub actor_user_id: Option<Uuid>,
    pub reason_code: Option<String>,
    pub occurred_at: DateTime<Utc>,
    pub trace_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalDraftPreview {
    pub id: Uuid,
    pub work_item_id: Uuid,
    pub finding_id: Uuid,
    pub draft_type: String,
    pub title: String,
    pub business_reason: String,
    pub requested_change_summary: String,
    pub impact_summary: String,
    pub source_snapshot_hash: String,
    pub preview_hash: String,
    pub created_by_user_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub consumed: bool,
    pub trace_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalDraft {
    pub id: Uuid,
    pub approval_draft_number: String,
    pub work_item_id: Uuid,
    pub finding_id: Uuid,
    pub action_code: String,
    pub draft_type: String,
    pub title: String,
    pub business_reason: String,
    pub primary_resource_type: String,
    pub primary_resource_id: String,
    pub requested_change_summary: String,
    pub before_snapshot: BTreeMap<String, String>,
    pub proposed_after_summary: String,
    pub impact_summary: String,
    pub required_approver_role_keys: Vec<String>,
    pub status: ApprovalDraftStatus,
    pub draft_only: bool,
    pub source_snapshot_hash: String,
    pub draft_hash: String,
    pub created_by_user_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub version: u64,
    pub trace_id: Uuid,
    pub resource_ref: ResourceRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetFindingLifecycleInput {
    pub finding_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetActionRecommendationsInput {
    pub finding_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetActionProposalInput {
    pub proposal_id: Uuid,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchWorkItemsInput {
    pub finding_id: Option<Uuid>,
    pub statuses: Option<Vec<WorkItemStatus>>,
    pub action_codes: Option<Vec<String>>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetWorkItemInput {
    pub work_item_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetApprovalDraftInput {
    pub approval_draft_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionReadResult<T> {
    pub schema_version: u8,
    pub status: String,
    pub items: Vec<T>,
    pub pagination: Option<Pagination>,
    pub data_classification: String,
    pub trace_id: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_statuses_are_draft_only() {
        let encoded = serde_json::to_string(&[
            ApprovalDraftStatus::Draft,
            ApprovalDraftStatus::ReadyForReview,
            ApprovalDraftStatus::Withdrawn,
            ApprovalDraftStatus::Expired,
            ApprovalDraftStatus::Superseded,
        ])
        .expect("status json");
        for forbidden in ["approved", "rejected", "executing", "executed", "posted"] {
            assert!(!encoded.contains(forbidden));
        }
    }

    #[test]
    fn agent_inputs_have_no_write_payloads() {
        let schema =
            serde_json::to_string(&schemars::schema_for!(SearchWorkItemsInput)).expect("schema");
        for forbidden in ["create", "update", "approve", "execute", "sql", "url"] {
            assert!(!schema.contains(forbidden));
        }
    }
}
