//! Canonical Markup Language (CML) task snapshots.
//!
//! CML is a deterministic, portable projection of signed Buzz task events. It
//! is not a concurrency authority; live state is reduced from Nostr events.

use std::collections::{BTreeMap, HashSet};

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::Sha256;
use thiserror::Error;
use uuid::Uuid;

/// Errors returned while parsing, validating, or serializing CML.
#[derive(Debug, Error)]
pub enum CmlError {
    /// The JSON representation is malformed or violates the strict schema.
    #[error("invalid CML JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// A semantic validation rule failed.
    #[error("invalid CML: {0}")]
    Validation(String),
}

/// A complete CML v1 task snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CmlTask {
    /// Acceptance criteria for the task.
    pub acceptance: Vec<AcceptanceCriterion>,
    /// Current blockers.
    pub blockers: Vec<Blocker>,
    /// Verification and delivery evidence.
    pub evidence: Vec<Evidence>,
    /// Git implementation identity.
    pub git: GitState,
    /// Stable task UUID.
    pub id: Uuid,
    /// Current exclusive claim, if any.
    pub lease: Option<Lease>,
    /// One testable outcome.
    pub objective: String,
    /// Task priority.
    pub priority: Priority,
    /// Protocol discriminator; must be `buzz-cml`.
    pub protocol: String,
    /// Reviewer/fixer round state.
    pub review: ReviewState,
    /// Assigned task roles.
    pub roles: Roles,
    /// Privacy-safe runtime projection.
    pub runtime: RuntimeState,
    /// Reduced task status.
    pub status: CmlStatus,
    /// Human-readable task title.
    pub title: String,
    /// Unix timestamp of the newest reduced transition.
    pub updated_at: u64,
    /// Schema version; must be 1.
    pub version: u8,
    /// Versioned extension namespace.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
}

/// One acceptance criterion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceCriterion {
    /// Stable criterion identifier unique within the task.
    pub id: String,
    /// Observable requirement.
    pub text: String,
    /// Whether an independent verifier accepted the criterion.
    pub verified: bool,
}

/// A task blocker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blocker {
    /// Stable blocker identifier.
    pub id: String,
    /// Human-readable blocker summary.
    pub text: String,
    /// Optional signed event or authorized URL reference.
    pub reference: Option<String>,
}

/// A verification evidence reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    /// Evidence category, such as `test`, `build`, `commit`, or `runtime`.
    pub kind: String,
    /// Content hash, event ID, commit SHA, or authorized URL.
    pub reference: String,
}

/// Git identity associated with a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitState {
    /// Upstream base commit.
    pub base_sha: String,
    /// Feature branch name.
    pub branch: String,
    /// Current implementation head, when one exists.
    pub head_sha: Option<String>,
    /// Repository in `owner/name` form.
    pub repo: String,
    /// Privacy-safe basename-like worktree alias.
    pub worktree_alias: String,
}

/// Exclusive task claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lease {
    /// Stable lease identifier.
    pub id: String,
    /// Agent pubkey holding the lease.
    pub holder: String,
    /// Unix issue timestamp.
    pub issued_at: u64,
    /// Unix expiration timestamp.
    pub expires_at: u64,
}

/// Reviewer/fixer loop state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewState {
    /// Hard v1 limit; must be exactly three.
    pub max_rounds: u8,
    /// Current round in the inclusive range 0..=3.
    pub round: u8,
}

/// Planner, worker, reviewer, and fixer assignments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Roles {
    /// Fixer pubkey.
    pub fixer: Option<String>,
    /// Planner pubkey.
    pub planner: String,
    /// Reviewer pubkey.
    pub reviewer: Option<String>,
    /// Worker pubkey.
    pub worker: Option<String>,
}

/// Privacy-safe task runtime projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeState {
    /// Stable pseudonymous host ID (`h_` plus lowercase hex), if assigned.
    pub host_id: Option<String>,
    /// Unix timestamp of the latest signed heartbeat.
    pub last_heartbeat_at: Option<u64>,
    /// Presence derived from heartbeat age and lease state.
    pub presence: Presence,
    /// Heartbeat TTL in seconds; v1 requires 180.
    pub ttl_seconds: u64,
}

/// CML task priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Priority {
    /// Urgent, release- or safety-blocking work.
    P0,
    /// High-priority work.
    P1,
    /// Normal-priority work.
    P2,
    /// Deferred work.
    P3,
}

/// Derived task status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CmlStatus {
    /// Proposed but not planned.
    Proposed,
    /// Planned and available for claim.
    Planned,
    /// Exclusively claimed.
    Claimed,
    /// Work is in progress.
    Working,
    /// Waiting on an external dependency or human.
    Blocked,
    /// Submitted for independent review.
    Review,
    /// Reviewer requested changes.
    Fixing,
    /// Independently verified.
    Verified,
    /// Merged into the integration branch.
    Integrated,
    /// Installed/running revision and behavior proved.
    Shipped,
    /// Cancelled before completion.
    Cancelled,
    /// Conflicting signed successors require resolution.
    Conflicted,
}

/// Presence derived from signed heartbeat age and lease state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Presence {
    /// Heartbeat is within the configured TTL.
    Online,
    /// Heartbeat is older than one TTL but no older than two TTLs.
    Stale,
    /// No sufficiently recent heartbeat exists.
    Offline,
}

/// Derive a stable channel-scoped pseudonymous host identifier.
///
/// The host secret remains local. Community, channel, and agent identity are
/// domain-separated in the HMAC so the same machine cannot be correlated across
/// channels and two agents on one machine receive different identifiers.
pub fn derive_host_id(
    host_secret: &[u8; 32],
    community_id: Uuid,
    channel_id: Uuid,
    agent_pubkey: &str,
) -> Result<String, CmlError> {
    validate_pubkey("agent_pubkey", agent_pubkey)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(host_secret)
        .map_err(|_| CmlError::Validation("invalid host secret".into()))?;
    mac.update(b"buzz-cml-host-id-v1\0");
    mac.update(community_id.as_bytes());
    mac.update(channel_id.as_bytes());
    mac.update(agent_pubkey.as_bytes());
    let digest = mac.finalize().into_bytes();
    Ok(format!("h_{}", hex::encode(&digest[..8])))
}

/// Parse and semantically validate a strict CML v1 document.
pub fn parse_cml(input: &str) -> Result<CmlTask, CmlError> {
    let task: CmlTask = serde_json::from_str(input)?;
    task.validate()?;
    Ok(task)
}

impl CmlTask {
    /// Validate all v1 semantic and privacy constraints.
    pub fn validate(&self) -> Result<(), CmlError> {
        if self.version != 1 || self.protocol != "buzz-cml" {
            return invalid("version must be 1 and protocol must be buzz-cml");
        }
        validate_nonempty("title", &self.title, 256)?;
        validate_nonempty("objective", &self.objective, 4096)?;
        if self.review.max_rounds != 3 || self.review.round > 3 {
            return invalid("review max_rounds must be 3 and round must be 0..=3");
        }
        if self.runtime.ttl_seconds != 180 {
            return invalid("runtime ttl_seconds must be 180 in CML v1");
        }
        validate_sha("base_sha", &self.git.base_sha)?;
        if let Some(head) = &self.git.head_sha {
            validate_sha("head_sha", head)?;
        }
        validate_repo(&self.git.repo)?;
        validate_branch(&self.git.branch)?;
        validate_worktree_alias(&self.git.worktree_alias)?;
        if let Some(host_id) = &self.runtime.host_id {
            validate_host_id(host_id)?;
        }
        validate_pubkey("planner", &self.roles.planner)?;
        for (name, value) in [
            ("worker", self.roles.worker.as_deref()),
            ("reviewer", self.roles.reviewer.as_deref()),
            ("fixer", self.roles.fixer.as_deref()),
        ] {
            if let Some(pubkey) = value {
                validate_pubkey(name, pubkey)?;
            }
        }
        let assigned_roles = [
            Some(self.roles.planner.as_str()),
            self.roles.worker.as_deref(),
            self.roles.reviewer.as_deref(),
            self.roles.fixer.as_deref(),
        ];
        let mut distinct_roles = HashSet::new();
        for pubkey in assigned_roles.into_iter().flatten() {
            if !distinct_roles.insert(pubkey) {
                return invalid("planner, worker, reviewer, and fixer must be distinct");
            }
        }
        if let Some(lease) = &self.lease {
            validate_nonempty("lease id", &lease.id, 128)?;
            validate_pubkey("lease holder", &lease.holder)?;
            if lease.expires_at <= lease.issued_at {
                return invalid("lease expires_at must be after issued_at");
            }
        }
        self.validate_presence()?;
        self.validate_status_lease()?;
        let mut acceptance_ids = HashSet::new();
        for criterion in &self.acceptance {
            validate_nonempty("acceptance id", &criterion.id, 64)?;
            validate_nonempty("acceptance text", &criterion.text, 4096)?;
            if !acceptance_ids.insert(&criterion.id) {
                return invalid("acceptance ids must be unique");
            }
        }
        for blocker in &self.blockers {
            validate_nonempty("blocker id", &blocker.id, 64)?;
            validate_nonempty("blocker text", &blocker.text, 4096)?;
            if let Some(reference) = &blocker.reference {
                validate_reference(reference)?;
            }
        }
        for evidence in &self.evidence {
            validate_nonempty("evidence kind", &evidence.kind, 64)?;
            validate_reference(&evidence.reference)?;
        }
        Ok(())
    }

    fn validate_presence(&self) -> Result<(), CmlError> {
        let expected = match self.runtime.last_heartbeat_at {
            None => Presence::Offline,
            Some(timestamp) => {
                let age = self.updated_at.checked_sub(timestamp).ok_or_else(|| {
                    CmlError::Validation("heartbeat cannot be in the future".into())
                })?;
                if age <= self.runtime.ttl_seconds {
                    Presence::Online
                } else if age <= self.runtime.ttl_seconds.saturating_mul(2) {
                    Presence::Stale
                } else {
                    Presence::Offline
                }
            }
        };
        if self.runtime.presence != expected {
            return invalid("presence must be derived from heartbeat age at updated_at");
        }
        Ok(())
    }

    fn validate_status_lease(&self) -> Result<(), CmlError> {
        let lease_holder = self.lease.as_ref().map(|lease| lease.holder.as_str());
        let expected_holder = match self.status {
            CmlStatus::Claimed | CmlStatus::Working | CmlStatus::Review => {
                self.roles.worker.as_deref()
            }
            CmlStatus::Fixing => self.roles.fixer.as_deref(),
            CmlStatus::Blocked => {
                if lease_holder == self.roles.fixer.as_deref() {
                    self.roles.fixer.as_deref()
                } else {
                    self.roles.worker.as_deref()
                }
            }
            CmlStatus::Proposed
            | CmlStatus::Planned
            | CmlStatus::Verified
            | CmlStatus::Integrated
            | CmlStatus::Shipped
            | CmlStatus::Cancelled
            | CmlStatus::Conflicted => {
                if self.lease.is_some() {
                    return invalid("status must not retain a task lease");
                }
                return Ok(());
            }
        };
        if expected_holder.is_none() || lease_holder != expected_holder {
            return invalid("status requires a lease held by its assigned worker or fixer");
        }
        Ok(())
    }

    /// Serialize as recursively key-sorted, two-space JSON with one final LF.
    pub fn to_canonical_json(&self) -> Result<String, CmlError> {
        self.validate()?;
        let value = serde_json::to_value(self)?;
        let sorted = sort_json(value);
        let mut output = serde_json::to_string_pretty(&sorted)?;
        output.push('\n');
        Ok(output)
    }
}

fn invalid<T>(message: &str) -> Result<T, CmlError> {
    Err(CmlError::Validation(message.to_owned()))
}

fn validate_nonempty(name: &str, value: &str, max_len: usize) -> Result<(), CmlError> {
    if value.trim().is_empty() || value.len() > max_len {
        return invalid(&format!(
            "{name} must be non-empty and at most {max_len} bytes"
        ));
    }
    Ok(())
}

fn validate_sha(name: &str, value: &str) -> Result<(), CmlError> {
    if value.len() != 40 || !is_lower_hex(value) {
        return invalid(&format!(
            "{name} must be 40 lowercase hexadecimal characters"
        ));
    }
    Ok(())
}

fn validate_pubkey(name: &str, value: &str) -> Result<(), CmlError> {
    if value.len() != 64 || !is_lower_hex(value) {
        return invalid(&format!(
            "{name} must be a 64-character lowercase hex pubkey"
        ));
    }
    Ok(())
}

fn validate_host_id(value: &str) -> Result<(), CmlError> {
    let digest = value.strip_prefix("h_").unwrap_or_default();
    if digest.len() < 16 || digest.len() > 64 || !is_lower_hex(digest) {
        return invalid("host_id must be h_ plus 16..64 lowercase hex characters");
    }
    Ok(())
}

fn validate_repo(value: &str) -> Result<(), CmlError> {
    let mut parts = value.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    if parts.next().is_some() || !is_safe_repo_component(owner) || !is_safe_repo_component(name) {
        return invalid("repo must use canonical owner/name form");
    }
    Ok(())
}

fn is_safe_repo_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn validate_branch(value: &str) -> Result<(), CmlError> {
    validate_nonempty("branch", value, 255)?;
    let forbidden = [' ', '~', '^', ':', '?', '*', '[', '\\'];
    if value == "."
        || value.starts_with('/')
        || value.ends_with('/')
        || value.ends_with('.')
        || value.ends_with(".lock")
        || value.contains("..")
        || value.contains("//")
        || value.contains("@{")
        || value.split('/').any(|part| part.starts_with('.'))
        || value
            .chars()
            .any(|character| character.is_control() || forbidden.contains(&character))
    {
        return invalid("branch is not a safe canonical ref name");
    }
    Ok(())
}

fn validate_worktree_alias(value: &str) -> Result<(), CmlError> {
    validate_nonempty("worktree_alias", value, 128)?;
    if value == "."
        || value == ".."
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return invalid("worktree_alias must be a portable basename-like token");
    }
    Ok(())
}

fn validate_reference(value: &str) -> Result<(), CmlError> {
    validate_nonempty("reference", value, 2048)?;
    if value.chars().any(char::is_control) {
        return invalid("reference must not contain control characters");
    }
    let is_hex_reference = matches!(value.len(), 40 | 64) && is_lower_hex(value);
    let is_https = value.starts_with("https://") && !value[8..].is_empty();
    let is_buzz = value.starts_with("buzz://") && !value[7..].is_empty();
    if !is_hex_reference && !is_https && !is_buzz {
        return invalid("reference must be a canonical hash, HTTPS URL, or Buzz URL");
    }
    Ok(())
}

fn is_lower_hex(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sort_json(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<_, _> = map
                .into_iter()
                .map(|(key, value)| (key, sort_json(value)))
                .collect();
            let object: Map<_, _> = sorted.into_iter().collect();
            Value::Object(object)
        }
        Value::Array(values) => Value::Array(values.into_iter().map(sort_json).collect()),
        scalar => scalar,
    }
}
