use crate::catalog::stable_uuid;
use crate::domain::ActionEngine;
use crate::helpers::*;
use crate::model::{ActionError, Actor, IdempotencyRecord};
use business_action_contracts::{
    ActionProposal, ConditionStatus, FindingLifecycle, ProposalStatus, ReviewStatus,
    WorkItemDraftStatus,
};
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

impl ActionEngine {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn review_finding(
        &mut self,
        actor: &Actor,
        id: Uuid,
        idempotency_key: &str,
        operation: &str,
        status: ReviewStatus,
        reason_code: Option<String>,
        review_after: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<FindingLifecycle, ActionError> {
        if !actor.can(business_action_contracts::FINDING_ACKNOWLEDGE) {
            return Err(ActionError::PermissionDenied);
        }
        self.finding(actor, id)?;
        validate_idempotency_key(idempotency_key)?;
        let request_hash = hash_value(&(id, operation, &reason_code, review_after))?;
        if let Some(existing_id) = self.idempotent_result(idempotency_key, &request_hash)? {
            return self
                .state
                .findings
                .get(&existing_id)
                .cloned()
                .ok_or(ActionError::IdempotencyConflict);
        }
        let finding = self
            .state
            .findings
            .get_mut(&id)
            .ok_or(ActionError::NotFoundOrForbidden)?;
        finding.review_status = status;
        finding.version = finding.version.saturating_add(1);
        finding.review_after = review_after;
        if status == ReviewStatus::Resolved {
            finding.resolved_at = Some(now);
        }
        if status == ReviewStatus::Dismissed {
            finding.dismissed_at = Some(now);
        }
        let result = finding.clone();
        self.state.idempotency.insert(
            idempotency_key.into(),
            IdempotencyRecord {
                user_id: actor.user_id,
                request_hash,
                entity_id: id,
            },
        );
        let event = match status {
            ReviewStatus::Acknowledged => "ANOMALY_FINDING_ACKNOWLEDGED",
            ReviewStatus::Resolved => "ANOMALY_FINDING_RESOLVED",
            ReviewStatus::Dismissed => "ANOMALY_FINDING_DISMISSED",
            _ => "ANOMALY_FINDING_REVIEW_CHANGED",
        };
        let status_name = format!("{status:?}").to_ascii_lowercase();
        self.audit(
            event,
            id,
            None,
            Some(&status_name),
            Some(result.finding_snapshot_hash.clone()),
            Some(actor.user_id),
            actor.trace_id,
            now,
        );
        Ok(result)
    }

    pub(crate) fn generate_proposals(
        &mut self,
        finding_id: Uuid,
        now: DateTime<Utc>,
        trace_id: Uuid,
    ) -> Result<(), ActionError> {
        let finding = self
            .state
            .findings
            .get(&finding_id)
            .cloned()
            .ok_or(ActionError::NotFoundOrForbidden)?;
        let entries = self
            .catalog
            .iter()
            .filter(|entry| {
                entry.enabled
                    && entry.effective_from <= now
                    && entry.effective_to.is_none_or(|end| end > now)
                    && entry
                        .supported_anomaly_types
                        .iter()
                        .any(|kind| kind == &finding.anomaly_type)
                    && entry
                        .target_resource_types
                        .iter()
                        .any(|kind| kind == &finding.primary_resource.r#type)
            })
            .cloned()
            .collect::<Vec<_>>();
        for entry in entries {
            if self.state.proposals.values().any(|proposal| {
                proposal.finding_id == finding_id
                    && proposal.action_code == entry.action_code
                    && proposal.finding_version == finding.version
            }) {
                continue;
            }
            let id = stable_uuid(
                format!(
                    "{}:{}:{}:{}",
                    finding.id, finding.version, entry.version, entry.action_code
                )
                .as_bytes(),
            );
            let mut proposal = ActionProposal {
                id,
                proposal_number: format!("AP-{}", &id.simple().to_string()[..12]),
                finding_id,
                action_catalog_version: entry.version.clone(),
                action_code: entry.action_code.clone(),
                title: entry.title,
                summary: entry.description,
                recommended_role_key: entry.allowed_assignee_role_keys.first().cloned(),
                default_due_at: now + Duration::days(i64::from(entry.default_due_days)),
                status: ProposalStatus::Suggested,
                finding_version: finding.version,
                finding_snapshot_hash: finding.finding_snapshot_hash.clone(),
                rule_set_version: finding.rule_set_version.clone(),
                proposal_hash: String::new(),
                created_at: now,
                expires_at: now + Duration::days(30),
                accepted_at: None,
                dismissed_at: None,
                created_by_type: "system".into(),
                created_by_id: "business-action-service".into(),
                trace_id,
                version: 1,
                resource_ref: resource("action_proposal", id, "action-proposal"),
            };
            proposal.proposal_hash = hash_value(&proposal)?;
            self.state.proposals.insert(id, proposal.clone());
            self.audit(
                "ACTION_PROPOSAL_GENERATED",
                id,
                Some(entry.action_code),
                Some("suggested"),
                Some(proposal.proposal_hash),
                None,
                trace_id,
                now,
            );
        }
        Ok(())
    }

    pub(crate) fn supersede_stale_proposals(
        &mut self,
        finding_id: Uuid,
        trace_id: Uuid,
        now: DateTime<Utc>,
    ) {
        let Some(finding) = self.state.findings.get(&finding_id) else {
            return;
        };
        let ids = self
            .state
            .proposals
            .values()
            .filter(|proposal| {
                proposal.finding_id == finding_id
                    && proposal.status == ProposalStatus::Suggested
                    && (proposal.finding_version != finding.version
                        || proposal.finding_snapshot_hash != finding.finding_snapshot_hash)
            })
            .map(|proposal| proposal.id)
            .collect::<Vec<_>>();
        for id in ids {
            let action_code = if let Some(proposal) = self.state.proposals.get_mut(&id) {
                proposal.status = ProposalStatus::Superseded;
                proposal.version = proposal.version.saturating_add(1);
                Some(proposal.action_code.clone())
            } else {
                None
            };
            self.audit(
                "ACTION_PROPOSAL_SUPERSEDED",
                id,
                action_code,
                Some("superseded"),
                None,
                None,
                trace_id,
                now,
            );
        }
    }

    pub(crate) fn expire(&mut self, now: DateTime<Utc>, trace_id: Uuid) {
        let mut reopened_ids = Vec::new();
        for finding in self.state.findings.values_mut() {
            if finding.review_status == ReviewStatus::Dismissed
                && finding.review_after.is_some_and(|at| at <= now)
            {
                finding.review_status = if finding.condition_status == ConditionStatus::Active {
                    reopened_ids.push(finding.id);
                    ReviewStatus::Reopened
                } else {
                    ReviewStatus::Unreviewed
                };
                finding.review_after = None;
                finding.version = finding.version.saturating_add(1);
            }
        }
        for id in reopened_ids {
            self.audit(
                "ANOMALY_FINDING_REOPENED",
                id,
                None,
                Some("reopened"),
                None,
                None,
                trace_id,
                now,
            );
            self.record_source_reactivated(id, trace_id, now);
        }
        let proposal_ids = self
            .state
            .proposals
            .values()
            .filter(|value| value.status == ProposalStatus::Suggested && value.expires_at <= now)
            .map(|value| value.id)
            .collect::<Vec<_>>();
        for id in proposal_ids {
            if let Some(value) = self.state.proposals.get_mut(&id) {
                value.status = ProposalStatus::Expired;
                value.version = value.version.saturating_add(1);
            }
        }
        let draft_ids = self
            .state
            .work_item_drafts
            .values()
            .filter(|value| {
                value.status == WorkItemDraftStatus::Prepared && value.expires_at <= now
            })
            .map(|value| value.id)
            .collect::<Vec<_>>();
        for id in draft_ids {
            if let Some(value) = self.state.work_item_drafts.get_mut(&id) {
                value.status = WorkItemDraftStatus::Expired;
            }
            self.audit(
                "WORK_ITEM_DRAFT_EXPIRED",
                id,
                None,
                Some("expired"),
                None,
                None,
                trace_id,
                now,
            );
        }
        let approval_ids = self
            .state
            .approval_drafts
            .values()
            .filter(|value| {
                matches!(
                    value.status,
                    business_action_contracts::ApprovalDraftStatus::Draft
                        | business_action_contracts::ApprovalDraftStatus::ReadyForReview
                ) && value.expires_at <= now
            })
            .map(|value| value.id)
            .collect::<Vec<_>>();
        for id in approval_ids {
            let action_code = if let Some(value) = self.state.approval_drafts.get_mut(&id) {
                value.status = business_action_contracts::ApprovalDraftStatus::Expired;
                value.updated_at = now;
                value.version = value.version.saturating_add(1);
                Some(value.action_code.clone())
            } else {
                None
            };
            self.audit(
                "APPROVAL_DRAFT_EXPIRED",
                id,
                action_code,
                Some("expired"),
                None,
                None,
                trace_id,
                now,
            );
        }
    }

    pub(crate) fn supersede_stale_approval_drafts(
        &mut self,
        finding_id: Uuid,
        trace_id: Uuid,
        now: DateTime<Utc>,
    ) {
        let Some(finding) = self.state.findings.get(&finding_id) else {
            return;
        };
        let ids = self
            .state
            .approval_drafts
            .values()
            .filter(|draft| {
                draft.finding_id == finding_id
                    && draft.source_snapshot_hash != finding.finding_snapshot_hash
                    && matches!(
                        draft.status,
                        business_action_contracts::ApprovalDraftStatus::Draft
                            | business_action_contracts::ApprovalDraftStatus::ReadyForReview
                    )
            })
            .map(|draft| draft.id)
            .collect::<Vec<_>>();
        for id in ids {
            let action_code = if let Some(draft) = self.state.approval_drafts.get_mut(&id) {
                draft.status = business_action_contracts::ApprovalDraftStatus::Superseded;
                draft.updated_at = now;
                draft.version = draft.version.saturating_add(1);
                Some(draft.action_code.clone())
            } else {
                None
            };
            self.audit(
                "APPROVAL_DRAFT_UPDATED",
                id,
                action_code,
                Some("superseded"),
                None,
                None,
                trace_id,
                now,
            );
        }
    }
}
