use crate::domain::ActionEngine;
use crate::helpers::{hash_value, resource, safe_comment, validate_idempotency_key};
use crate::model::{
    ActionError, Actor, ConfirmApprovalDraft, IdempotencyRecord, PrepareApprovalDraft,
    UpdateApprovalDraft,
};
use business_action_contracts::{ApprovalDraft, ApprovalDraftPreview, ApprovalDraftStatus};
use std::collections::BTreeMap;
use uuid::Uuid;

impl ActionEngine {
    pub fn prepare_approval_draft(
        &mut self,
        actor: &Actor,
        input: PrepareApprovalDraft,
    ) -> Result<ApprovalDraftPreview, ActionError> {
        if !actor.can(business_action_contracts::APPROVAL_DRAFT_CREATE) {
            return Err(ActionError::PermissionDenied);
        }
        if !safe_comment(&input.business_reason, 1, 1000)
            || !safe_comment(&input.requested_change_summary, 1, 1000)
            || !safe_comment(&input.impact_summary, 1, 1000)
        {
            return Err(ActionError::InvalidRequest);
        }
        let item = self
            .state
            .work_items
            .get(&input.work_item_id)
            .cloned()
            .ok_or(ActionError::NotFoundOrForbidden)?;
        self.finding(actor, item.finding_id)?;
        if !item.status.is_active() {
            return Err(ActionError::InvalidTransition);
        }
        let catalog = self.catalog_entry(&item.action_code, input.now)?;
        let draft_type = catalog
            .approval_draft_type
            .clone()
            .ok_or(ActionError::ApprovalDraftNotSupported)?;
        let id = Uuid::new_v4();
        let mut preview = ApprovalDraftPreview {
            id,
            work_item_id: item.id,
            finding_id: item.finding_id,
            draft_type,
            title: format!("{}审批材料草稿", catalog.title),
            business_reason: input.business_reason,
            requested_change_summary: input.requested_change_summary,
            impact_summary: input.impact_summary,
            source_snapshot_hash: item.finding_snapshot_hash,
            preview_hash: String::new(),
            created_by_user_id: actor.user_id,
            created_at: input.now,
            expires_at: input.now + self.work_item_draft_ttl,
            consumed: false,
            trace_id: actor.trace_id,
        };
        preview.preview_hash = hash_value(&preview)?;
        self.state.approval_previews.insert(id, preview.clone());
        self.audit(
            "APPROVAL_DRAFT_PREPARED",
            id,
            Some(item.action_code),
            Some("prepared"),
            Some(preview.preview_hash.clone()),
            Some(actor.user_id),
            actor.trace_id,
            input.now,
        );
        Ok(preview)
    }

    pub fn confirm_approval_draft(
        &mut self,
        actor: &Actor,
        input: ConfirmApprovalDraft,
    ) -> Result<ApprovalDraft, ActionError> {
        if !actor.can(business_action_contracts::APPROVAL_DRAFT_CREATE) {
            return Err(ActionError::PermissionDenied);
        }
        validate_idempotency_key(&input.idempotency_key)?;
        let request_hash = hash_value(&(
            input.preview_id,
            &input.preview_hash,
            input.expected_work_item_version,
        ))?;
        if let Some(id) = self.idempotent_result(&input.idempotency_key, &request_hash)? {
            return self
                .state
                .approval_drafts
                .get(&id)
                .cloned()
                .ok_or(ActionError::IdempotencyConflict);
        }
        let preview = self
            .state
            .approval_previews
            .get(&input.preview_id)
            .cloned()
            .ok_or(ActionError::StalePreview)?;
        if preview.consumed {
            return Err(ActionError::PreviewConsumed);
        }
        if preview.expires_at <= input.now {
            return Err(ActionError::PreviewExpired);
        }
        if preview.preview_hash != input.preview_hash {
            return Err(ActionError::StalePreview);
        }
        let item = self
            .state
            .work_items
            .get(&preview.work_item_id)
            .cloned()
            .ok_or(ActionError::StalePreview)?;
        let finding = self.finding(actor, item.finding_id)?.clone();
        if item.version != input.expected_work_item_version
            || item.finding_snapshot_hash != preview.source_snapshot_hash
            || !item.status.is_active()
        {
            return Err(ActionError::StalePreview);
        }
        self.state.approval_draft_sequence = self.state.approval_draft_sequence.saturating_add(1);
        let id = Uuid::new_v4();
        let mut draft = ApprovalDraft {
            id,
            approval_draft_number: format!("AD-{:06}", self.state.approval_draft_sequence),
            work_item_id: item.id,
            finding_id: finding.id,
            action_code: item.action_code.clone(),
            draft_type: preview.draft_type,
            title: preview.title,
            business_reason: preview.business_reason,
            primary_resource_type: finding.primary_resource.r#type.clone(),
            primary_resource_id: finding.primary_resource.id.clone().unwrap_or_default(),
            requested_change_summary: preview.requested_change_summary,
            before_snapshot: BTreeMap::from([
                (
                    "findingSnapshotHash".into(),
                    finding.finding_snapshot_hash.clone(),
                ),
                (
                    "conditionStatus".into(),
                    format!("{:?}", finding.condition_status).to_ascii_lowercase(),
                ),
            ]),
            proposed_after_summary: "仅供正式审批流程准备；本草稿不改变业务状态。".into(),
            impact_summary: preview.impact_summary,
            required_approver_role_keys: vec!["business_owner".into(), "finance_reviewer".into()],
            status: ApprovalDraftStatus::Draft,
            draft_only: true,
            source_snapshot_hash: finding.finding_snapshot_hash,
            draft_hash: String::new(),
            created_by_user_id: actor.user_id,
            created_at: input.now,
            updated_at: input.now,
            expires_at: input.now + self.approval_draft_ttl,
            version: 1,
            trace_id: actor.trace_id,
            resource_ref: resource("approval_draft", id, "approval-draft"),
        };
        draft.draft_hash = hash_value(&draft)?;
        self.state.approval_drafts.insert(id, draft.clone());
        self.state.idempotency.insert(
            input.idempotency_key,
            IdempotencyRecord {
                user_id: actor.user_id,
                request_hash,
                entity_id: id,
            },
        );
        if let Some(preview) = self.state.approval_previews.get_mut(&input.preview_id) {
            preview.consumed = true;
        }
        self.audit(
            "APPROVAL_DRAFT_CREATED",
            id,
            Some(item.action_code),
            Some("draft"),
            Some(draft.draft_hash.clone()),
            Some(actor.user_id),
            actor.trace_id,
            input.now,
        );
        Ok(draft)
    }

    pub fn update_approval_draft(
        &mut self,
        actor: &Actor,
        input: UpdateApprovalDraft,
    ) -> Result<ApprovalDraft, ActionError> {
        if !actor.can(business_action_contracts::APPROVAL_DRAFT_CREATE) {
            return Err(ActionError::PermissionDenied);
        }
        let existing = self
            .state
            .approval_drafts
            .get(&input.approval_draft_id)
            .cloned()
            .ok_or(ActionError::NotFoundOrForbidden)?;
        self.finding(actor, existing.finding_id)?;
        if existing.version != input.expected_version {
            return Err(ActionError::VersionConflict);
        }
        if !matches!(
            (existing.status, input.status),
            (
                ApprovalDraftStatus::Draft,
                ApprovalDraftStatus::ReadyForReview
            ) | (ApprovalDraftStatus::Draft, ApprovalDraftStatus::Withdrawn)
                | (
                    ApprovalDraftStatus::ReadyForReview,
                    ApprovalDraftStatus::Withdrawn
                )
        ) {
            return Err(ActionError::InvalidTransition);
        }
        for value in [
            input.business_reason.as_deref(),
            input.requested_change_summary.as_deref(),
            input.impact_summary.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if !safe_comment(value, 1, 1000) {
                return Err(ActionError::InvalidRequest);
            }
        }
        let mut updated = existing;
        updated.status = input.status;
        if let Some(value) = input.business_reason {
            updated.business_reason = value;
        }
        if let Some(value) = input.requested_change_summary {
            updated.requested_change_summary = value;
        }
        if let Some(value) = input.impact_summary {
            updated.impact_summary = value;
        }
        updated.updated_at = input.now;
        updated.version = updated.version.saturating_add(1);
        updated.draft_hash = hash_value(&updated)?;
        self.state
            .approval_drafts
            .insert(updated.id, updated.clone());
        let status_name = format!("{:?}", updated.status).to_ascii_lowercase();
        self.audit(
            if input.status == ApprovalDraftStatus::Withdrawn {
                "APPROVAL_DRAFT_WITHDRAWN"
            } else {
                "APPROVAL_DRAFT_UPDATED"
            },
            updated.id,
            Some(updated.action_code.clone()),
            Some(&status_name),
            Some(updated.draft_hash.clone()),
            Some(actor.user_id),
            actor.trace_id,
            input.now,
        );
        Ok(updated)
    }
}
