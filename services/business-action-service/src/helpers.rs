use crate::model::ActionError;
use business_action_contracts::{
    ConditionStatus, FindingLifecycle, FindingScope, ReviewStatus, WorkItemStatus,
};
use business_anomaly_contracts::BusinessAnomaly;
use business_query_contracts::ResourceRef;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[allow(clippy::too_many_arguments)]
pub(crate) fn lifecycle_from(
    id: Uuid,
    finding_key: String,
    observation: crate::model::FindingObservation,
    scope_hash: &str,
    rule_set_version: &str,
    snapshot_hash: String,
    now: DateTime<Utc>,
    trace_id: Uuid,
) -> FindingLifecycle {
    let finding = observation.finding;
    FindingLifecycle {
        id,
        finding_key,
        anomaly_type: finding.r#type,
        severity: finding.severity,
        confidence: finding.confidence,
        title: finding.title,
        summary_code: finding.summary_code,
        primary_resource: finding.primary_resource,
        related_resources: finding.related_resources,
        impact: finding.impact,
        rule: finding.rule,
        evidence_summary: finding
            .evidence
            .into_iter()
            .take(10)
            .map(|value| format!("{}:{}:{}", value.object_type, value.object_id, value.field))
            .collect(),
        data_as_of: finding.data_as_of,
        scope: observation.scope,
        scope_hash: scope_hash.into(),
        rule_set_version: rule_set_version.into(),
        condition_status: ConditionStatus::Active,
        review_status: ReviewStatus::Unreviewed,
        occurrence_count: 1,
        first_seen_at: now,
        last_seen_at: now,
        cleared_at: None,
        resolved_at: None,
        dismissed_at: None,
        review_after: None,
        finding_snapshot_hash: snapshot_hash,
        version: 1,
        trace_id,
    }
}

pub(crate) fn copy_finding_snapshot(
    target: &mut FindingLifecycle,
    source: &BusinessAnomaly,
    scope: &FindingScope,
) {
    target.anomaly_type.clone_from(&source.r#type);
    target.severity = source.severity.clone();
    target.confidence = source.confidence.clone();
    target.title.clone_from(&source.title);
    target.summary_code.clone_from(&source.summary_code);
    target.primary_resource.clone_from(&source.primary_resource);
    target
        .related_resources
        .clone_from(&source.related_resources);
    target.impact.clone_from(&source.impact);
    target.rule.clone_from(&source.rule);
    target.data_as_of = source.data_as_of;
    target.scope.clone_from(scope);
    target.evidence_summary = source
        .evidence
        .iter()
        .take(10)
        .map(|value| format!("{}:{}:{}", value.object_type, value.object_id, value.field))
        .collect();
}

pub(crate) fn finding_key(
    finding: &BusinessAnomaly,
    scope: &FindingScope,
) -> Result<String, ActionError> {
    let primary_id = finding
        .primary_resource
        .id
        .as_deref()
        .ok_or(ActionError::InvalidRequest)?;
    hash_value(&(
        &finding.rule.id,
        &finding.primary_resource.r#type,
        primary_id,
        &scope.legal_entity_id,
        &scope.warehouse_id,
        &scope.customer_id,
        &scope.supplier_id,
        &scope.brand_id,
        &scope.business_unit_id,
    ))
}

pub(crate) fn finding_snapshot_hash(
    finding: &BusinessAnomaly,
    scope: &FindingScope,
    scope_hash: &str,
    rule_set_version: &str,
) -> Result<String, ActionError> {
    hash_value(&(finding, scope, scope_hash, rule_set_version))
}

pub(crate) fn hash_value<T: Serialize>(value: &T) -> Result<String, ActionError> {
    let bytes = serde_json::to_vec(value).map_err(|_| ActionError::InvalidRequest)?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

pub(crate) fn validate_idempotency_key(value: &str) -> Result<(), ActionError> {
    if (16..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        Ok(())
    } else {
        Err(ActionError::InvalidRequest)
    }
}

pub(crate) fn safe_comment(value: &str, min: usize, max: usize) -> bool {
    let value = value.trim();
    (min..=max).contains(&value.len())
        && !value.chars().any(|character| character.is_control())
        && ![
            "begin private key",
            "access_token",
            "session_token",
            "<script",
            "javascript:",
        ]
        .iter()
        .any(|forbidden| value.to_ascii_lowercase().contains(forbidden))
}

pub(crate) fn resource(kind: &str, id: Uuid, slug: &str) -> ResourceRef {
    ResourceRef {
        r#type: kind.into(),
        id: Some(id.to_string()),
        title: format!("打开 {}", id.simple()),
        biz_uri: format!("biz://{slug}/{id}"),
    }
}

pub(crate) fn allowed_transition(from: WorkItemStatus, to: WorkItemStatus) -> bool {
    matches!(
        (from, to),
        (WorkItemStatus::Open, WorkItemStatus::InProgress)
            | (WorkItemStatus::InProgress, WorkItemStatus::ReadyForReview)
            | (WorkItemStatus::ReadyForReview, WorkItemStatus::Completed)
            | (WorkItemStatus::Open, WorkItemStatus::Blocked)
            | (WorkItemStatus::InProgress, WorkItemStatus::Blocked)
            | (WorkItemStatus::Blocked, WorkItemStatus::InProgress)
            | (WorkItemStatus::Open, WorkItemStatus::Cancelled)
            | (WorkItemStatus::InProgress, WorkItemStatus::Cancelled)
            | (WorkItemStatus::Blocked, WorkItemStatus::Cancelled)
            | (WorkItemStatus::Completed, WorkItemStatus::Reopened)
            | (WorkItemStatus::Reopened, WorkItemStatus::InProgress)
    )
}

pub(crate) fn event_for_status(from: WorkItemStatus, status: WorkItemStatus) -> &'static str {
    if from == WorkItemStatus::Blocked && status == WorkItemStatus::InProgress {
        return "unblocked";
    }
    match status {
        WorkItemStatus::InProgress => "started",
        WorkItemStatus::Blocked => "blocked",
        WorkItemStatus::ReadyForReview => "ready_for_review",
        WorkItemStatus::Completed => "completed",
        WorkItemStatus::Cancelled => "cancelled",
        WorkItemStatus::Reopened => "reopened",
        WorkItemStatus::Open => "created",
    }
}
