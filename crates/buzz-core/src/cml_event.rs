//! Signed CML task-event validation and deterministic reduction.

use std::collections::{HashMap, HashSet};

use nostr::{Event, EventId};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    cml::{parse_cml, CmlError, CmlStatus, CmlTask},
    kind::{
        KIND_JOB_ACCEPTED, KIND_JOB_CANCEL, KIND_JOB_PROGRESS, KIND_JOB_REQUEST, KIND_JOB_RESULT,
    },
    verification::verify_event,
    VerificationError,
};

/// Errors returned while validating or reducing signed CML events.
#[derive(Debug, Error)]
pub enum CmlEventError {
    /// Event signature or identifier verification failed.
    #[error("invalid signed event: {0}")]
    Verification(#[from] VerificationError),
    /// Embedded CML failed strict parsing or semantic validation.
    #[error("invalid CML snapshot: {0}")]
    Cml(#[from] CmlError),
    /// Tags, actor, kind, chain, or transition are invalid.
    #[error("invalid CML event: {0}")]
    Invalid(String),
}

/// A signed CML lifecycle transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmlTransition {
    /// Planner publishes the executable plan.
    Plan,
    /// Worker accepts an exclusive claim.
    Claim,
    /// Worker begins implementation.
    Start,
    /// Worker submits for review.
    Submit,
    /// Reviewer requests fixes.
    ReviewReject,
    /// Fixer submits corrections.
    FixSubmit,
    /// Reviewer accepts the implementation.
    ReviewApprove,
    /// Integrator records the merge.
    Merge,
    /// Independent verifier records installed runtime proof.
    RuntimeProve,
    /// Worker/fixer reports a blocker.
    Block,
    /// Planner cancels the task.
    Cancel,
    /// Planner records expiration of an exclusive lease.
    LeaseExpired,
    /// Authorized resolver selects one side of a fork.
    OwnerResolve,
}

impl CmlTransition {
    /// Wire-format transition name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "planner.plan",
            Self::Claim => "worker.claim",
            Self::Start => "worker.start",
            Self::Submit => "worker.submit",
            Self::ReviewReject => "reviewer.reject",
            Self::FixSubmit => "fixer.submit",
            Self::ReviewApprove => "reviewer.approve",
            Self::Merge => "integrator.merge",
            Self::RuntimeProve => "runtime.prove",
            Self::Block => "worker.block",
            Self::Cancel => "planner.cancel",
            Self::LeaseExpired => "planner.lease-expired",
            Self::OwnerResolve => "owner.resolve",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "planner.plan" => Self::Plan,
            "worker.claim" => Self::Claim,
            "worker.start" => Self::Start,
            "worker.submit" => Self::Submit,
            "reviewer.reject" => Self::ReviewReject,
            "fixer.submit" => Self::FixSubmit,
            "reviewer.approve" => Self::ReviewApprove,
            "integrator.merge" => Self::Merge,
            "runtime.prove" => Self::RuntimeProve,
            "worker.block" => Self::Block,
            "planner.cancel" => Self::Cancel,
            "planner.lease-expired" => Self::LeaseExpired,
            "owner.resolve" => Self::OwnerResolve,
            _ => return None,
        })
    }

    /// Existing Buzz event kind used by this transition.
    pub const fn event_kind(self) -> u32 {
        match self {
            Self::Plan => KIND_JOB_REQUEST,
            Self::Claim => KIND_JOB_ACCEPTED,
            Self::ReviewApprove | Self::RuntimeProve => KIND_JOB_RESULT,
            Self::Cancel => KIND_JOB_CANCEL,
            Self::OwnerResolve
            | Self::Start
            | Self::Submit
            | Self::ReviewReject
            | Self::FixSubmit
            | Self::Merge
            | Self::Block
            | Self::LeaseExpired => KIND_JOB_PROGRESS,
        }
    }

    /// Actor role authorized to publish this transition.
    pub const fn actor_role(self) -> CmlRole {
        match self {
            Self::Plan | Self::Merge | Self::Cancel | Self::LeaseExpired | Self::OwnerResolve => {
                CmlRole::Planner
            }
            Self::Claim | Self::Start | Self::Submit | Self::Block => CmlRole::Worker,
            Self::ReviewReject | Self::ReviewApprove | Self::RuntimeProve => CmlRole::Reviewer,
            Self::FixSubmit => CmlRole::Fixer,
        }
    }

    /// Whether an actor role may publish this transition.
    pub const fn allows_role(self, role: CmlRole) -> bool {
        match self {
            Self::Block => matches!(role, CmlRole::Worker | CmlRole::Fixer),
            _ => role as u8 == self.actor_role() as u8,
        }
    }

    fn expected_kind(self) -> u32 {
        self.event_kind()
    }
}

/// Actor role asserted by a signed transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CmlRole {
    /// Planner/owner role.
    Planner,
    /// Worker role.
    Worker,
    /// Independent reviewer role.
    Reviewer,
    /// Fixer role.
    Fixer,
}

impl CmlRole {
    /// Wire-format role name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Planner => "planner",
            Self::Worker => "worker",
            Self::Reviewer => "reviewer",
            Self::Fixer => "fixer",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "planner" => Self::Planner,
            "worker" => Self::Worker,
            "reviewer" => Self::Reviewer,
            "fixer" => Self::Fixer,
            _ => return None,
        })
    }
}

/// A fully validated signed CML transition.
#[derive(Debug, Clone)]
pub struct ValidatedCmlEvent {
    /// Original signed event identifier.
    pub id: EventId,
    /// Channel UUID from the `h` tag.
    pub channel_id: Uuid,
    /// Strict task snapshot embedded in event content.
    pub task: CmlTask,
    /// Lifecycle transition.
    pub transition: CmlTransition,
    /// Actor role.
    pub role: CmlRole,
    /// Immediate predecessor event, absent for a plan root and fork resolution.
    pub previous: Option<EventId>,
    /// Two fork heads referenced by an `owner.resolve` transition.
    pub fork_heads: Vec<EventId>,
    /// Fork head selected by an `owner.resolve` transition.
    pub selected: Option<EventId>,
}

/// Result of deterministic task-event reduction.
#[derive(Debug, Clone)]
pub struct ReducedCmlTask {
    /// Reduced task projection.
    pub task: CmlTask,
    /// Current event head, or the common predecessor when conflicted.
    pub head: EventId,
    /// True when multiple valid successors exist and no resolution was applied.
    pub conflicted: bool,
}

/// Verify signature, strict tags, canonical CML, actor, kind, and status agreement.
pub fn validate_cml_event(event: &Event) -> Result<ValidatedCmlEvent, CmlEventError> {
    verify_event(event)?;
    validate_cml_event_after_signature(event)
}

/// Validate strict CML semantics after the caller has already verified the signature.
///
/// Relay ingest uses this to avoid repeating its CPU-bound Schnorr verification.
pub fn validate_cml_event_after_signature(
    event: &Event,
) -> Result<ValidatedCmlEvent, CmlEventError> {
    let channel_id = parse_uuid_tag(event, "h")?;
    let task_id = parse_uuid_tag(event, "d")?;
    let protocol = exact_tag(event, "protocol")?;
    if protocol.len() != 3 || protocol[1] != "buzz-cml" || protocol[2] != "1" {
        return invalid("protocol tag must be [protocol,buzz-cml,1]");
    }
    let transition = CmlTransition::parse(exact_value(event, "transition")?)
        .ok_or_else(|| CmlEventError::Invalid("unknown transition".into()))?;
    let role = CmlRole::parse(exact_value(event, "role")?)
        .ok_or_else(|| CmlEventError::Invalid("unknown role".into()))?;
    if u32::from(event.kind.as_u16()) != transition.expected_kind() {
        return invalid("event kind does not match transition");
    }
    let task = parse_cml(&event.content)?;
    if task.to_canonical_json()? != event.content {
        return invalid("event content must be canonical CML");
    }
    if task.id != task_id {
        return invalid("d tag must equal embedded task id");
    }
    if task.updated_at != event.created_at.as_secs() {
        return invalid("snapshot updated_at must equal event created_at");
    }
    if exact_value(event, "status")? != status_name(task.status) {
        return invalid("status tag must match embedded task status");
    }
    if !transition.allows_role(role) {
        return invalid("role does not match transition");
    }
    let expected_actor = role_pubkey(&task, role)
        .ok_or_else(|| CmlEventError::Invalid("transition role is unassigned".into()))?;
    if event.pubkey.to_hex() != expected_actor {
        return invalid("event author does not match assigned role");
    }
    let previous = marker_event_id(event, "prev")?;
    let fork_a = marker_event_id(event, "fork_a")?;
    let fork_b = marker_event_id(event, "fork_b")?;
    let selected = marker_event_id(event, "selected")?;
    let fork_heads = match (fork_a, fork_b, selected) {
        (None, None, None) if transition != CmlTransition::OwnerResolve => Vec::new(),
        (Some(a), Some(b), Some(selected)) if transition == CmlTransition::OwnerResolve => {
            if a == b || (selected != a && selected != b) {
                return invalid("fork resolution must select one of two distinct heads");
            }
            vec![a, b]
        }
        _ => return invalid("fork markers are complete and exclusive to owner.resolve"),
    };
    if transition == CmlTransition::Plan {
        if previous.is_some() {
            return invalid("plan root must not have a predecessor");
        }
    } else if transition != CmlTransition::OwnerResolve && previous.is_none() {
        return invalid("non-root transition requires exactly one predecessor");
    }
    if transition == CmlTransition::OwnerResolve && previous.is_some() {
        return invalid("owner.resolve uses fork markers instead of prev");
    }
    Ok(ValidatedCmlEvent {
        id: event.id,
        channel_id,
        task,
        transition,
        role,
        previous,
        fork_heads,
        selected,
    })
}

/// Reduce a task's signed events independently of input ordering.
pub fn reduce_cml_events(events: &[Event]) -> Result<ReducedCmlTask, CmlEventError> {
    if events.is_empty() {
        return invalid("cannot reduce an empty event set");
    }
    let validated: Vec<_> = events
        .iter()
        .map(validate_cml_event)
        .collect::<Result<_, _>>()?;
    let task_id = validated[0].task.id;
    let channel_id = validated[0].channel_id;
    if validated
        .iter()
        .any(|event| event.task.id != task_id || event.channel_id != channel_id)
    {
        return invalid("all reduced events must share task and channel ids");
    }
    let roots: Vec<_> = validated
        .iter()
        .filter(|event| event.transition == CmlTransition::Plan && event.previous.is_none())
        .collect();
    if roots.len() != 1 {
        return invalid("event set must contain exactly one plan root");
    }
    let mut by_id: HashMap<EventId, &ValidatedCmlEvent> = HashMap::new();
    for event in &validated {
        if by_id.insert(event.id, event).is_some() {
            return invalid("duplicate event id");
        }
    }
    let mut current = roots[0];
    let mut visited = HashSet::from([current.id]);
    loop {
        let children: Vec<_> = validated
            .iter()
            .filter(|event| event.previous == Some(current.id))
            .collect();
        match children.as_slice() {
            [] => {
                return Ok(ReducedCmlTask {
                    task: current.task.clone(),
                    head: current.id,
                    conflicted: false,
                })
            }
            [child] => {
                validate_transition(current, child)?;
                if !visited.insert(child.id) {
                    return invalid("event chain contains a cycle");
                }
                current = child;
            }
            _ => {
                let child_ids: HashSet<_> = children.iter().map(|child| child.id).collect();
                let resolutions: Vec<_> = validated
                    .iter()
                    .filter(|event| {
                        event.transition == CmlTransition::OwnerResolve
                            && event.fork_heads.iter().copied().collect::<HashSet<_>>() == child_ids
                            && event
                                .selected
                                .is_some_and(|selected| child_ids.contains(&selected))
                    })
                    .collect();
                if let [resolution] = resolutions.as_slice() {
                    let selected_id = resolution.selected.ok_or_else(|| {
                        CmlEventError::Invalid("resolution missing selected head".into())
                    })?;
                    let selected = by_id.get(&selected_id).copied().ok_or_else(|| {
                        CmlEventError::Invalid("selected fork head is absent".into())
                    })?;
                    validate_transition(current, selected)?;
                    validate_resolution_snapshot(selected, resolution)?;
                    for child in &children {
                        visited.insert(child.id);
                    }
                    if !visited.insert(resolution.id) {
                        return invalid("event chain contains a cycle");
                    }
                    current = resolution;
                    continue;
                }
                let mut task = current.task.clone();
                task.status = CmlStatus::Conflicted;
                task.lease = None;
                return Ok(ReducedCmlTask {
                    task,
                    head: current.id,
                    conflicted: true,
                });
            }
        }
    }
}

fn validate_resolution_snapshot(
    selected: &ValidatedCmlEvent,
    resolution: &ValidatedCmlEvent,
) -> Result<(), CmlEventError> {
    let mut expected = selected.task.clone();
    expected.updated_at = resolution.task.updated_at;
    if expected != resolution.task {
        return invalid("owner.resolve may only advance updated_at on the selected snapshot");
    }
    Ok(())
}

fn validate_transition(
    previous: &ValidatedCmlEvent,
    current: &ValidatedCmlEvent,
) -> Result<(), CmlEventError> {
    let valid = matches!(
        (
            previous.task.status,
            current.transition,
            current.task.status
        ),
        (CmlStatus::Planned, CmlTransition::Claim, CmlStatus::Claimed)
            | (CmlStatus::Claimed, CmlTransition::Start, CmlStatus::Working)
            | (CmlStatus::Working, CmlTransition::Submit, CmlStatus::Review)
            | (CmlStatus::Working, CmlTransition::Block, CmlStatus::Blocked)
            | (
                CmlStatus::Review,
                CmlTransition::ReviewReject,
                CmlStatus::Fixing
            )
            | (
                CmlStatus::Review,
                CmlTransition::ReviewApprove,
                CmlStatus::Verified
            )
            | (
                CmlStatus::Fixing,
                CmlTransition::FixSubmit,
                CmlStatus::Review
            )
            | (CmlStatus::Fixing, CmlTransition::Block, CmlStatus::Blocked)
            | (
                CmlStatus::Verified,
                CmlTransition::Merge,
                CmlStatus::Integrated
            )
            | (
                CmlStatus::Integrated,
                CmlTransition::RuntimeProve,
                CmlStatus::Shipped
            )
            | (
                CmlStatus::Claimed,
                CmlTransition::LeaseExpired,
                CmlStatus::Planned
            )
    ) || (current.transition == CmlTransition::Cancel
        && current.task.status == CmlStatus::Cancelled
        && !matches!(
            previous.task.status,
            CmlStatus::Shipped | CmlStatus::Cancelled
        ));
    if !valid {
        return invalid("invalid lifecycle transition");
    }
    if current.task.id != previous.task.id {
        return invalid("task id changed across transition");
    }
    validate_immutable_contract(&previous.task, &current.task)?;
    if current.transition == CmlTransition::ReviewReject {
        if previous.task.review.round.checked_add(1) != Some(current.task.review.round) {
            return invalid("review rejection must increment round by exactly one");
        }
    } else if current.task.review.round != previous.task.review.round {
        return invalid("only reviewer.reject may advance the review round");
    }
    Ok(())
}

fn validate_immutable_contract(previous: &CmlTask, current: &CmlTask) -> Result<(), CmlEventError> {
    let unchanged = previous.title == current.title
        && previous.objective == current.objective
        && previous.priority == current.priority
        && previous.protocol == current.protocol
        && previous.version == current.version
        && acceptance_contract_equal(&previous.acceptance, &current.acceptance)
        && previous.roles == current.roles
        && previous.git.base_sha == current.git.base_sha
        && previous.git.branch == current.git.branch
        && previous.git.repo == current.git.repo
        && previous.git.worktree_alias == current.git.worktree_alias
        && previous.extensions == current.extensions;
    if !unchanged {
        return invalid("signed planner contract changed after planning");
    }
    Ok(())
}

fn acceptance_contract_equal(
    previous: &[crate::cml::AcceptanceCriterion],
    current: &[crate::cml::AcceptanceCriterion],
) -> bool {
    previous.len() == current.len()
        && previous
            .iter()
            .zip(current)
            .all(|(left, right)| left.id == right.id && left.text == right.text)
}

fn role_pubkey(task: &CmlTask, role: CmlRole) -> Option<&str> {
    match role {
        CmlRole::Planner => Some(task.roles.planner.as_str()),
        CmlRole::Worker => task.roles.worker.as_deref(),
        CmlRole::Reviewer => task.roles.reviewer.as_deref(),
        CmlRole::Fixer => task.roles.fixer.as_deref(),
    }
}

fn exact_tag<'a>(event: &'a Event, name: &str) -> Result<&'a [String], CmlEventError> {
    let matches: Vec<_> = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some(name))
        .collect();
    if matches.len() != 1 {
        return invalid("required tag must occur exactly once");
    }
    Ok(matches[0].as_slice())
}

fn exact_value<'a>(event: &'a Event, name: &str) -> Result<&'a str, CmlEventError> {
    let tag = exact_tag(event, name)?;
    if tag.len() != 2 {
        return invalid("single-value tag has wrong arity");
    }
    Ok(tag[1].as_str())
}

fn parse_uuid_tag(event: &Event, name: &str) -> Result<Uuid, CmlEventError> {
    Uuid::parse_str(exact_value(event, name)?)
        .map_err(|_| CmlEventError::Invalid(format!("{name} tag must be a canonical UUID")))
}

fn marker_event_id(event: &Event, marker: &str) -> Result<Option<EventId>, CmlEventError> {
    let mut matches = event.tags.iter().filter(|tag| {
        let values = tag.as_slice();
        values.len() == 3 && values[0] == "e" && values[2] == marker
    });
    let first = matches.next();
    if matches.next().is_some() {
        return invalid("event marker must occur at most once");
    }
    first
        .map(|tag| {
            EventId::from_hex(&tag.as_slice()[1])
                .map_err(|_| CmlEventError::Invalid("event marker id must be hex".into()))
        })
        .transpose()
}

fn status_name(status: CmlStatus) -> &'static str {
    match status {
        CmlStatus::Proposed => "proposed",
        CmlStatus::Planned => "planned",
        CmlStatus::Claimed => "claimed",
        CmlStatus::Working => "working",
        CmlStatus::Blocked => "blocked",
        CmlStatus::Review => "review",
        CmlStatus::Fixing => "fixing",
        CmlStatus::Verified => "verified",
        CmlStatus::Integrated => "integrated",
        CmlStatus::Shipped => "shipped",
        CmlStatus::Cancelled => "cancelled",
        CmlStatus::Conflicted => "conflicted",
    }
}

fn invalid<T>(message: &str) -> Result<T, CmlEventError> {
    Err(CmlEventError::Invalid(message.to_owned()))
}
