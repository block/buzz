use crate::catalog::stable_uuid;
use crate::helpers::*;
use crate::model::{
    ActionError, Actor, AssigneeResolver, AuditEvent, ConfirmWorkItem,
    CurrentUserOnlyAssigneeResolver, FindingObservation, IdempotencyRecord, PrepareWorkItem,
    UpdateWorkItem,
};
use business_action_contracts::{
    ActionCatalogEntry, ActionProposal, ApprovalDraft, ApprovalDraftPreview, ConditionStatus,
    DismissReasonCode, FindingLifecycle, ProposalStatus, ResolutionCode, ReviewStatus, RunStatus,
    WorkItem, WorkItemDraft, WorkItemDraftStatus, WorkItemEvent, WorkItemStatus,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use uuid::Uuid;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActionState {
    pub findings: BTreeMap<Uuid, FindingLifecycle>,
    pub proposals: BTreeMap<Uuid, ActionProposal>,
    pub work_item_drafts: BTreeMap<Uuid, WorkItemDraft>,
    pub work_items: BTreeMap<Uuid, WorkItem>,
    pub work_item_events: BTreeMap<Uuid, WorkItemEvent>,
    pub approval_previews: BTreeMap<Uuid, ApprovalDraftPreview>,
    pub approval_drafts: BTreeMap<Uuid, ApprovalDraft>,
    pub audits: Vec<AuditEvent>,
    pub(crate) idempotency: BTreeMap<String, IdempotencyRecord>,
    pub(crate) work_item_sequence: u64,
    pub(crate) approval_draft_sequence: u64,
}

pub struct ActionEngine {
    pub state: ActionState,
    pub(crate) catalog: Vec<ActionCatalogEntry>,
    pub(crate) work_item_draft_ttl: Duration,
    pub(crate) approval_draft_ttl: Duration,
    pub(crate) max_active_items_per_finding: usize,
    pub(crate) assignee_resolver: Arc<dyn AssigneeResolver>,
}

impl ActionEngine {
    pub fn new(catalog: Vec<ActionCatalogEntry>) -> Result<Self, ActionError> {
        if catalog.is_empty() {
            return Err(ActionError::InvalidRequest);
        }
        Ok(Self {
            state: ActionState::default(),
            catalog,
            work_item_draft_ttl: Duration::minutes(10),
            approval_draft_ttl: Duration::days(7),
            max_active_items_per_finding: 5,
            assignee_resolver: Arc::new(CurrentUserOnlyAssigneeResolver),
        })
    }

    pub fn from_state(
        catalog: Vec<ActionCatalogEntry>,
        state: ActionState,
    ) -> Result<Self, ActionError> {
        let mut engine = Self::new(catalog)?;
        engine.state = state;
        Ok(engine)
    }

    pub fn catalog(&self) -> &[ActionCatalogEntry] {
        &self.catalog
    }

    pub fn configure_limits(
        &mut self,
        work_item_draft_ttl: Duration,
        approval_draft_ttl: Duration,
        max_active_items_per_finding: usize,
    ) -> Result<(), ActionError> {
        if work_item_draft_ttl <= Duration::zero()
            || approval_draft_ttl <= Duration::zero()
            || max_active_items_per_finding == 0
        {
            return Err(ActionError::InvalidRequest);
        }
        self.work_item_draft_ttl = work_item_draft_ttl;
        self.approval_draft_ttl = approval_draft_ttl;
        self.max_active_items_per_finding = max_active_items_per_finding;
        Ok(())
    }

    pub fn ingest_run(
        &mut self,
        observations: Vec<FindingObservation>,
        run_status: RunStatus,
        scope_hash: &str,
        rule_set_version: &str,
        now: DateTime<Utc>,
        trace_id: Uuid,
    ) -> Result<Vec<Uuid>, ActionError> {
        if scope_hash.len() != 64 || rule_set_version.is_empty() {
            return Err(ActionError::InvalidRequest);
        }
        self.expire(now, trace_id);
        let mut seen = BTreeSet::new();
        let mut ids = Vec::with_capacity(observations.len());
        for observation in observations {
            let finding_key = finding_key(&observation.finding, &observation.scope)?;
            let id = stable_uuid(finding_key.as_bytes());
            seen.insert(id);
            let snapshot_hash = finding_snapshot_hash(
                &observation.finding,
                &observation.scope,
                scope_hash,
                rule_set_version,
            )?;
            let mut reopened = false;
            if let Some(existing) = self.state.findings.get_mut(&id) {
                let changed = existing.finding_snapshot_hash != snapshot_hash;
                existing.occurrence_count = existing.occurrence_count.saturating_add(1);
                existing.last_seen_at = now;
                existing.condition_status = ConditionStatus::Active;
                existing.cleared_at = None;
                if matches!(
                    existing.review_status,
                    ReviewStatus::Resolved | ReviewStatus::Dismissed
                ) {
                    existing.review_status = ReviewStatus::Reopened;
                    existing.review_after = None;
                    reopened = true;
                }
                if changed || reopened {
                    existing.version = existing.version.saturating_add(1);
                    existing.finding_snapshot_hash = snapshot_hash.clone();
                    copy_finding_snapshot(existing, &observation.finding, &observation.scope);
                }
            } else {
                self.state.findings.insert(
                    id,
                    lifecycle_from(
                        id,
                        finding_key,
                        observation,
                        scope_hash,
                        rule_set_version,
                        snapshot_hash.clone(),
                        now,
                        trace_id,
                    ),
                );
            }
            if reopened {
                self.audit(
                    "ANOMALY_FINDING_REOPENED",
                    id,
                    None,
                    Some("reopened"),
                    Some(snapshot_hash.clone()),
                    None,
                    trace_id,
                    now,
                );
                self.record_source_reactivated(id, trace_id, now);
            }
            self.supersede_stale_proposals(id, trace_id, now);
            self.supersede_stale_approval_drafts(id, trace_id, now);
            self.generate_proposals(id, now, trace_id)?;
            ids.push(id);
        }
        if run_status == RunStatus::Completed {
            let clear_ids = self
                .state
                .findings
                .values()
                .filter(|finding| {
                    finding.scope_hash == scope_hash
                        && finding.rule_set_version == rule_set_version
                        && finding.condition_status == ConditionStatus::Active
                        && !seen.contains(&finding.id)
                })
                .map(|finding| finding.id)
                .collect::<Vec<_>>();
            for id in clear_ids {
                if let Some(finding) = self.state.findings.get_mut(&id) {
                    finding.condition_status = ConditionStatus::Cleared;
                    finding.cleared_at = Some(now);
                    finding.version = finding.version.saturating_add(1);
                }
                self.update_source_condition(id, ConditionStatus::Cleared, trace_id, now);
                self.audit(
                    "ANOMALY_FINDING_CLEARED",
                    id,
                    None,
                    Some("cleared"),
                    None,
                    None,
                    trace_id,
                    now,
                );
            }
        }
        Ok(ids)
    }

    pub fn finding(&self, actor: &Actor, id: Uuid) -> Result<&FindingLifecycle, ActionError> {
        let finding = self
            .state
            .findings
            .get(&id)
            .ok_or(ActionError::NotFoundOrForbidden)?;
        actor
            .can_read(finding)
            .then_some(finding)
            .ok_or(ActionError::NotFoundOrForbidden)
    }

    pub fn proposals(
        &self,
        actor: &Actor,
        finding_id: Uuid,
    ) -> Result<Vec<&ActionProposal>, ActionError> {
        self.finding(actor, finding_id)?;
        if !actor.can(business_action_contracts::ACTION_PROPOSAL_READ) {
            return Err(ActionError::NotFoundOrForbidden);
        }
        Ok(self
            .state
            .proposals
            .values()
            .filter(|proposal| proposal.finding_id == finding_id)
            .collect())
    }

    pub fn proposal(&self, actor: &Actor, id: Uuid) -> Result<&ActionProposal, ActionError> {
        let proposal = self
            .state
            .proposals
            .get(&id)
            .ok_or(ActionError::NotFoundOrForbidden)?;
        self.finding(actor, proposal.finding_id)?;
        actor
            .can(business_action_contracts::ACTION_PROPOSAL_READ)
            .then_some(proposal)
            .ok_or(ActionError::NotFoundOrForbidden)
    }

    pub fn dismiss_proposal(
        &mut self,
        actor: &Actor,
        id: Uuid,
        idempotency_key: &str,
        now: DateTime<Utc>,
    ) -> Result<ActionProposal, ActionError> {
        if !actor.can(business_action_contracts::FINDING_ACKNOWLEDGE) {
            return Err(ActionError::PermissionDenied);
        }
        let proposal = self.proposal(actor, id)?.clone();
        if proposal.status != ProposalStatus::Suggested {
            return Err(ActionError::InvalidTransition);
        }
        validate_idempotency_key(idempotency_key)?;
        let request_hash = hash_value(&(id, "dismiss_proposal"))?;
        if let Some(existing_id) = self.idempotent_result(idempotency_key, &request_hash)? {
            return self
                .state
                .proposals
                .get(&existing_id)
                .cloned()
                .ok_or(ActionError::IdempotencyConflict);
        }
        let updated = self
            .state
            .proposals
            .get_mut(&id)
            .ok_or(ActionError::NotFoundOrForbidden)?;
        updated.status = ProposalStatus::Dismissed;
        updated.dismissed_at = Some(now);
        updated.version = updated.version.saturating_add(1);
        let updated = updated.clone();
        self.state.idempotency.insert(
            idempotency_key.into(),
            IdempotencyRecord {
                user_id: actor.user_id,
                request_hash,
                entity_id: id,
            },
        );
        self.audit(
            "ACTION_PROPOSAL_DISMISSED",
            id,
            Some(updated.action_code.clone()),
            Some("dismissed"),
            Some(updated.proposal_hash.clone()),
            Some(actor.user_id),
            actor.trace_id,
            now,
        );
        Ok(updated)
    }

    pub fn acknowledge(
        &mut self,
        actor: &Actor,
        id: Uuid,
        idempotency_key: &str,
        now: DateTime<Utc>,
    ) -> Result<FindingLifecycle, ActionError> {
        self.review_finding(
            actor,
            id,
            idempotency_key,
            "acknowledge",
            ReviewStatus::Acknowledged,
            None,
            None,
            now,
        )
    }

    pub fn resolve(
        &mut self,
        actor: &Actor,
        id: Uuid,
        idempotency_key: &str,
        code: ResolutionCode,
        note: &str,
        now: DateTime<Utc>,
    ) -> Result<FindingLifecycle, ActionError> {
        if !safe_comment(note, 1, 500) {
            return Err(ActionError::InvalidRequest);
        }
        self.review_finding(
            actor,
            id,
            idempotency_key,
            "resolve",
            ReviewStatus::Resolved,
            Some(format!("{code:?}").to_ascii_lowercase()),
            None,
            now,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn dismiss(
        &mut self,
        actor: &Actor,
        id: Uuid,
        idempotency_key: &str,
        code: DismissReasonCode,
        comment: &str,
        review_after: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<FindingLifecycle, ActionError> {
        if !safe_comment(comment, 1, 500)
            || review_after <= now
            || review_after > now + Duration::days(365)
        {
            return Err(ActionError::InvalidRequest);
        }
        self.review_finding(
            actor,
            id,
            idempotency_key,
            "dismiss",
            ReviewStatus::Dismissed,
            Some(format!("{code:?}").to_ascii_lowercase()),
            Some(review_after),
            now,
        )
    }

    pub fn prepare_work_item(
        &mut self,
        actor: &Actor,
        input: PrepareWorkItem,
    ) -> Result<WorkItemDraft, ActionError> {
        if !actor.can(business_action_contracts::WORK_ITEM_CREATE) {
            return Err(ActionError::PermissionDenied);
        }
        let proposal = self.proposal(actor, input.proposal_id)?.clone();
        if proposal.status != ProposalStatus::Suggested || proposal.expires_at <= input.now {
            return Err(ActionError::StalePreview);
        }
        let finding = self.finding(actor, proposal.finding_id)?.clone();
        let catalog = self
            .catalog_entry(&proposal.action_code, input.now)?
            .clone();
        self.assignee_resolver.validate(
            actor,
            &catalog,
            input.assignee_user_id,
            input.assignee_role_key.as_deref(),
        )?;
        let id = Uuid::new_v4();
        let mut draft = WorkItemDraft {
            id,
            proposal_id: proposal.id,
            finding_id: finding.id,
            title: catalog.title.clone(),
            description: format!(
                "{}（异常摘要：{}）",
                catalog.description, finding.summary_code
            ),
            assignee_user_id: input.assignee_user_id,
            assignee_role_key: input.assignee_role_key,
            due_at: input.now + Duration::days(i64::from(catalog.default_due_days)),
            priority: input.priority,
            finding_snapshot_hash: finding.finding_snapshot_hash.clone(),
            expected_finding_version: finding.version,
            preview_hash: String::new(),
            created_by_user_id: actor.user_id,
            created_at: input.now,
            expires_at: input.now + self.work_item_draft_ttl,
            status: WorkItemDraftStatus::Prepared,
            trace_id: actor.trace_id,
        };
        draft.preview_hash = hash_value(&draft)?;
        self.state.work_item_drafts.insert(id, draft.clone());
        self.audit(
            "WORK_ITEM_DRAFT_PREPARED",
            id,
            Some(catalog.action_code),
            Some("prepared"),
            Some(draft.preview_hash.clone()),
            Some(actor.user_id),
            actor.trace_id,
            input.now,
        );
        Ok(draft)
    }

    pub fn confirm_work_item(
        &mut self,
        actor: &Actor,
        input: ConfirmWorkItem,
    ) -> Result<WorkItem, ActionError> {
        if !actor.can(business_action_contracts::WORK_ITEM_CREATE) {
            return Err(ActionError::PermissionDenied);
        }
        validate_idempotency_key(&input.idempotency_key)?;
        let request_hash = hash_value(&(
            input.draft_id,
            &input.preview_hash,
            input.expected_finding_version,
        ))?;
        if let Some(id) = self.idempotent_result(&input.idempotency_key, &request_hash)? {
            return self
                .state
                .work_items
                .get(&id)
                .cloned()
                .ok_or(ActionError::IdempotencyConflict);
        }
        let draft = self
            .state
            .work_item_drafts
            .get(&input.draft_id)
            .cloned()
            .ok_or(ActionError::StalePreview)?;
        if draft.status != WorkItemDraftStatus::Prepared {
            return Err(ActionError::PreviewConsumed);
        }
        if draft.expires_at <= input.now {
            return Err(ActionError::PreviewExpired);
        }
        if draft.preview_hash != input.preview_hash {
            return Err(ActionError::StalePreview);
        }
        let proposal = self.proposal(actor, draft.proposal_id)?.clone();
        let finding = self.finding(actor, draft.finding_id)?.clone();
        if proposal.status != ProposalStatus::Suggested
            || finding.version != input.expected_finding_version
            || finding.version != draft.expected_finding_version
            || finding.finding_snapshot_hash != draft.finding_snapshot_hash
        {
            return Err(ActionError::StalePreview);
        }
        let catalog = self
            .catalog_entry(&proposal.action_code, input.now)?
            .clone();
        self.assignee_resolver.validate(
            actor,
            &catalog,
            draft.assignee_user_id,
            draft.assignee_role_key.as_deref(),
        )?;
        if let Some(existing) = self.state.work_items.values().find(|item| {
            item.finding_id == finding.id
                && item.action_code == proposal.action_code
                && item.status.is_active()
        }) {
            return Ok(existing.clone());
        }
        let active_count = self
            .state
            .work_items
            .values()
            .filter(|item| item.finding_id == finding.id && item.status.is_active())
            .count();
        if active_count >= self.max_active_items_per_finding {
            return Err(ActionError::InvalidRequest);
        }
        self.state.work_item_sequence = self.state.work_item_sequence.saturating_add(1);
        let id = Uuid::new_v4();
        let item = WorkItem {
            id,
            work_item_number: format!("WI-{:06}", self.state.work_item_sequence),
            finding_id: finding.id,
            proposal_id: proposal.id,
            action_code: proposal.action_code.clone(),
            title: draft.title,
            description: draft.description,
            priority: draft.priority,
            status: WorkItemStatus::Open,
            assignee_user_id: draft.assignee_user_id,
            assignee_role_key: draft.assignee_role_key,
            created_by_user_id: actor.user_id,
            due_at: draft.due_at,
            started_at: None,
            completed_at: None,
            cancelled_at: None,
            source_condition_status: finding.condition_status,
            finding_snapshot_hash: finding.finding_snapshot_hash,
            created_at: input.now,
            updated_at: input.now,
            version: 1,
            trace_id: actor.trace_id,
            resource_ref: resource("work_item", id, "work-item"),
        };
        self.state.work_items.insert(id, item.clone());
        self.state.idempotency.insert(
            input.idempotency_key,
            IdempotencyRecord {
                user_id: actor.user_id,
                request_hash,
                entity_id: id,
            },
        );
        if let Some(draft) = self.state.work_item_drafts.get_mut(&input.draft_id) {
            draft.status = WorkItemDraftStatus::Consumed;
        }
        if let Some(proposal) = self.state.proposals.get_mut(&proposal.id) {
            proposal.status = ProposalStatus::Accepted;
            proposal.accepted_at = Some(input.now);
            proposal.version = proposal.version.saturating_add(1);
        }
        if let Some(finding) = self.state.findings.get_mut(&finding.id) {
            finding.review_status = ReviewStatus::InProgress;
        }
        self.work_item_event(
            &item,
            "created",
            None,
            Some(WorkItemStatus::Open),
            actor,
            None,
            input.now,
        );
        self.audit(
            "WORK_ITEM_CREATED",
            id,
            Some(item.action_code.clone()),
            Some("open"),
            Some(item.finding_snapshot_hash.clone()),
            Some(actor.user_id),
            actor.trace_id,
            input.now,
        );
        Ok(item)
    }

    pub fn update_work_item(
        &mut self,
        actor: &Actor,
        input: UpdateWorkItem,
    ) -> Result<WorkItem, ActionError> {
        if !actor.can(business_action_contracts::WORK_ITEM_UPDATE) {
            return Err(ActionError::PermissionDenied);
        }
        let existing = self
            .state
            .work_items
            .get(&input.work_item_id)
            .cloned()
            .ok_or(ActionError::NotFoundOrForbidden)?;
        self.finding(actor, existing.finding_id)?;
        if existing.version != input.expected_version {
            return Err(ActionError::VersionConflict);
        }
        if !allowed_transition(existing.status, input.status) {
            return Err(ActionError::InvalidTransition);
        }
        let catalog = self
            .catalog_entry(&existing.action_code, input.now)?
            .clone();
        self.assignee_resolver.validate(
            actor,
            &catalog,
            input.assignee_user_id,
            input.assignee_role_key.as_deref(),
        )?;
        if (input.assignee_user_id != existing.assignee_user_id
            || input.assignee_role_key != existing.assignee_role_key)
            && !actor.can(business_action_contracts::WORK_ITEM_ASSIGN)
        {
            return Err(ActionError::PermissionDenied);
        }
        if input.status == WorkItemStatus::Completed
            && !actor.can(business_action_contracts::WORK_ITEM_COMPLETE)
        {
            return Err(ActionError::PermissionDenied);
        }
        let assignee_changed = input.assignee_user_id != existing.assignee_user_id
            || input.assignee_role_key != existing.assignee_role_key;
        let mut updated = existing.clone();
        updated.status = input.status;
        updated.assignee_user_id = input.assignee_user_id;
        updated.assignee_role_key = input.assignee_role_key;
        updated.updated_at = input.now;
        updated.version = updated.version.saturating_add(1);
        if input.status == WorkItemStatus::InProgress && updated.started_at.is_none() {
            updated.started_at = Some(input.now);
        }
        if input.status == WorkItemStatus::Completed {
            updated.completed_at = Some(input.now);
        }
        if input.status == WorkItemStatus::Cancelled {
            updated.cancelled_at = Some(input.now);
        }
        self.state.work_items.insert(updated.id, updated.clone());
        if assignee_changed {
            self.work_item_event(
                &updated,
                "assigned",
                Some(existing.status),
                Some(input.status),
                actor,
                None,
                input.now,
            );
            self.audit(
                "WORK_ITEM_ASSIGNED",
                updated.id,
                Some(updated.action_code.clone()),
                Some("assigned"),
                None,
                Some(actor.user_id),
                actor.trace_id,
                input.now,
            );
        }
        self.work_item_event(
            &updated,
            event_for_status(existing.status, input.status),
            Some(existing.status),
            Some(input.status),
            actor,
            input.reason_code,
            input.now,
        );
        let status_name = format!("{:?}", updated.status).to_ascii_lowercase();
        self.audit(
            if input.status == WorkItemStatus::Completed {
                "WORK_ITEM_COMPLETED"
            } else if input.status == WorkItemStatus::Cancelled {
                "WORK_ITEM_CANCELLED"
            } else if input.status == WorkItemStatus::Reopened {
                "WORK_ITEM_REOPENED"
            } else {
                "WORK_ITEM_STATUS_CHANGED"
            },
            updated.id,
            Some(updated.action_code.clone()),
            Some(&status_name),
            None,
            Some(actor.user_id),
            actor.trace_id,
            input.now,
        );
        Ok(updated)
    }

    pub fn block_business_write(
        &mut self,
        actor: Option<&Actor>,
        operation: &str,
        now: DateTime<Utc>,
        trace_id: Uuid,
    ) -> ActionError {
        self.audit(
            "BUSINESS_WRITE_ATTEMPT_BLOCKED",
            stable_uuid(operation.as_bytes()),
            None,
            Some("blocked"),
            None,
            actor.map(|value| value.user_id),
            trace_id,
            now,
        );
        ActionError::BusinessWriteNotAvailable
    }

    fn update_source_condition(
        &mut self,
        finding_id: Uuid,
        status: ConditionStatus,
        trace_id: Uuid,
        now: DateTime<Utc>,
    ) {
        let ids = self
            .state
            .work_items
            .values()
            .filter(|item| item.finding_id == finding_id)
            .map(|item| item.id)
            .collect::<Vec<_>>();
        for id in ids {
            let snapshot = if let Some(item) = self.state.work_items.get_mut(&id) {
                item.source_condition_status = status;
                item.updated_at = now;
                item.version = item.version.saturating_add(1);
                Some(item.clone())
            } else {
                None
            };
            if let Some(item) = snapshot {
                let actor = Actor {
                    user_id: Uuid::nil(),
                    permissions: BTreeSet::new(),
                    authorized_scope: business_analytics::AuthorizationScope::default(),
                    trace_id,
                };
                self.work_item_event(
                    &item,
                    "source_condition_changed",
                    Some(item.status),
                    Some(item.status),
                    &actor,
                    Some(format!("source_{status:?}").to_ascii_lowercase()),
                    now,
                );
            }
        }
    }

    pub(crate) fn record_source_reactivated(
        &mut self,
        finding_id: Uuid,
        trace_id: Uuid,
        now: DateTime<Utc>,
    ) {
        let ids = self
            .state
            .work_items
            .values()
            .filter(|item| {
                item.finding_id == finding_id && item.status == WorkItemStatus::Completed
            })
            .map(|item| item.id)
            .collect::<Vec<_>>();
        for id in ids {
            if let Some(item) = self.state.work_items.get(&id).cloned() {
                let actor = Actor {
                    user_id: Uuid::nil(),
                    permissions: BTreeSet::new(),
                    authorized_scope: business_analytics::AuthorizationScope::default(),
                    trace_id,
                };
                self.work_item_event(
                    &item,
                    "source_reactivated",
                    Some(item.status),
                    Some(item.status),
                    &actor,
                    Some("finding_reappeared".into()),
                    now,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn work_item_event(
        &mut self,
        item: &WorkItem,
        event_type: &str,
        from: Option<WorkItemStatus>,
        to: Option<WorkItemStatus>,
        actor: &Actor,
        reason: Option<String>,
        now: DateTime<Utc>,
    ) {
        let id = Uuid::new_v4();
        self.state.work_item_events.insert(
            id,
            WorkItemEvent {
                id,
                work_item_id: item.id,
                event_type: event_type.into(),
                from_status: from,
                to_status: to,
                actor_user_id: (!actor.user_id.is_nil()).then_some(actor.user_id),
                reason_code: reason,
                occurred_at: now,
                trace_id: actor.trace_id,
            },
        );
    }

    pub(crate) fn idempotent_result(
        &self,
        key: &str,
        request_hash: &str,
    ) -> Result<Option<Uuid>, ActionError> {
        match self.state.idempotency.get(key) {
            Some(record) if record.request_hash == request_hash => Ok(Some(record.entity_id)),
            Some(_) => Err(ActionError::IdempotencyConflict),
            None => Ok(None),
        }
    }

    pub(crate) fn catalog_entry(
        &self,
        action_code: &str,
        now: DateTime<Utc>,
    ) -> Result<&ActionCatalogEntry, ActionError> {
        self.catalog
            .iter()
            .find(|entry| {
                entry.action_code == action_code
                    && entry.enabled
                    && entry.effective_from <= now
                    && entry.effective_to.is_none_or(|end| end > now)
            })
            .ok_or(ActionError::InvalidActionCode)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn audit(
        &mut self,
        event_type: &str,
        entity_id: Uuid,
        action_code: Option<String>,
        status: Option<&str>,
        hash: Option<String>,
        user_id: Option<Uuid>,
        trace_id: Uuid,
        now: DateTime<Utc>,
    ) {
        self.state.audits.push(AuditEvent {
            id: Uuid::new_v4(),
            event_type: event_type.into(),
            result: "success".into(),
            entity_id: Some(entity_id),
            action_code,
            status: status.map(str::to_owned),
            hash,
            user_id,
            reason_code: None,
            version: None,
            trace_id,
            occurred_at: now,
        });
    }
}
