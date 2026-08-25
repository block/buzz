use business_action_contracts::{
    ActionCatalogEntry, ApprovalDraftStatus, DismissReasonCode, FindingLifecycle, FindingScope,
    Priority, ResolutionCode, WorkItemStatus,
};
use business_analytics::AuthorizationScope;
use business_anomaly_contracts::BusinessAnomaly;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEvent {
    pub id: Uuid,
    pub event_type: String,
    pub result: String,
    pub entity_id: Option<Uuid>,
    pub action_code: Option<String>,
    pub status: Option<String>,
    pub hash: Option<String>,
    pub user_id: Option<Uuid>,
    pub reason_code: Option<String>,
    pub version: Option<u64>,
    pub trace_id: Uuid,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct IdempotencyRecord {
    pub(crate) user_id: Uuid,
    pub(crate) request_hash: String,
    pub(crate) entity_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct FindingObservation {
    pub finding: BusinessAnomaly,
    pub scope: FindingScope,
}

#[derive(Debug, Clone)]
pub struct Actor {
    pub user_id: Uuid,
    pub permissions: BTreeSet<String>,
    pub authorized_scope: AuthorizationScope,
    pub trace_id: Uuid,
}

impl Actor {
    pub fn can(&self, permission: &str) -> bool {
        self.permissions.contains(permission)
    }

    pub(crate) fn can_read(&self, finding: &FindingLifecycle) -> bool {
        self.can(business_action_contracts::FINDING_READ)
            && self
                .authorized_scope
                .allows_legal_entity(&finding.scope.legal_entity_id)
            && finding
                .scope
                .warehouse_id
                .as_deref()
                .is_none_or(|value| self.authorized_scope.allows_warehouse(value))
            && finding
                .scope
                .customer_id
                .as_deref()
                .is_none_or(|value| self.authorized_scope.allows_customer(value))
            && finding
                .scope
                .supplier_id
                .as_deref()
                .is_none_or(|value| self.authorized_scope.allows_supplier(value))
            && finding
                .scope
                .brand_id
                .as_deref()
                .is_none_or(|value| self.authorized_scope.allows_brand(value))
            && finding
                .scope
                .business_unit_id
                .as_deref()
                .is_none_or(|value| {
                    self.authorized_scope.business_unit_ids.is_empty()
                        || self.authorized_scope.business_unit_ids.contains(value)
                })
    }
}

pub trait AssigneeResolver: Send + Sync {
    fn validate(
        &self,
        actor: &Actor,
        catalog: &ActionCatalogEntry,
        user_id: Option<Uuid>,
        role: Option<&str>,
    ) -> Result<(), ActionError>;
}

pub struct CurrentUserOnlyAssigneeResolver;

impl AssigneeResolver for CurrentUserOnlyAssigneeResolver {
    fn validate(
        &self,
        actor: &Actor,
        catalog: &ActionCatalogEntry,
        user_id: Option<Uuid>,
        role: Option<&str>,
    ) -> Result<(), ActionError> {
        if user_id.is_some_and(|id| id != actor.user_id)
            || role.is_some_and(|value| {
                !catalog
                    .allowed_assignee_role_keys
                    .iter()
                    .any(|allowed| allowed == value)
            })
        {
            return Err(ActionError::AssigneeNotAllowed);
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct PrepareWorkItem {
    pub proposal_id: Uuid,
    pub assignee_user_id: Option<Uuid>,
    pub assignee_role_key: Option<String>,
    pub priority: Priority,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ConfirmWorkItem {
    pub draft_id: Uuid,
    pub preview_hash: String,
    pub idempotency_key: String,
    pub expected_finding_version: u64,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpdateWorkItem {
    pub work_item_id: Uuid,
    pub expected_version: u64,
    pub status: WorkItemStatus,
    pub assignee_user_id: Option<Uuid>,
    pub assignee_role_key: Option<String>,
    pub reason_code: Option<String>,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct PrepareApprovalDraft {
    pub work_item_id: Uuid,
    pub business_reason: String,
    pub requested_change_summary: String,
    pub impact_summary: String,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ConfirmApprovalDraft {
    pub preview_id: Uuid,
    pub preview_hash: String,
    pub idempotency_key: String,
    pub expected_work_item_version: u64,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpdateApprovalDraft {
    pub approval_draft_id: Uuid,
    pub expected_version: u64,
    pub status: ApprovalDraftStatus,
    pub business_reason: Option<String>,
    pub requested_change_summary: Option<String>,
    pub impact_summary: Option<String>,
    pub now: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ActionError {
    #[error("not_found_or_forbidden")]
    NotFoundOrForbidden,
    #[error("permission_denied")]
    PermissionDenied,
    #[error("invalid_request")]
    InvalidRequest,
    #[error("invalid_action_code")]
    InvalidActionCode,
    #[error("stale_preview")]
    StalePreview,
    #[error("preview_expired")]
    PreviewExpired,
    #[error("preview_consumed")]
    PreviewConsumed,
    #[error("idempotency_conflict")]
    IdempotencyConflict,
    #[error("version_conflict")]
    VersionConflict,
    #[error("invalid_transition")]
    InvalidTransition,
    #[error("assignee_not_allowed")]
    AssigneeNotAllowed,
    #[error("approval_draft_not_supported")]
    ApprovalDraftNotSupported,
    #[error("business_write_not_available")]
    BusinessWriteNotAvailable,
    #[error("persistence_unavailable")]
    PersistenceUnavailable,
}

#[allow(dead_code)]
fn _contract_markers(_: DismissReasonCode, _: ResolutionCode) {}
