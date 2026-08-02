use serde::{de::MapAccess, de::Visitor, Deserialize, Deserializer, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

use super::ManagedAgentProcess;

const MAX_RECOVERY_RELAY_URL_BYTES: usize = 4096;
const TERMINAL_PROOF_PENDING_CLEAR_DETAIL: &str =
    "terminal proof established; recovery clear pending";
const PENDING_PAIR_FAILURE_PREFIX: &str = "pending managed-agent pair failures: ";

fn recovery_projection_present(error: &str) -> bool {
    error
        .lines()
        .any(|line| line.starts_with(super::UNVERIFIED_JOB_REAP_PREFIX))
}

fn pending_pair_failure_line(error: Option<&str>) -> Option<String> {
    error?
        .lines()
        .find(|line| line.starts_with(PENDING_PAIR_FAILURE_PREFIX))
        .map(str::to_string)
}

pub(crate) fn pending_pair_failures(
    record: &super::ManagedAgentRecord,
) -> BTreeMap<String, String> {
    pending_pair_failure_line(record.last_error.as_deref())
        .and_then(|line| {
            line.strip_prefix(PENDING_PAIR_FAILURE_PREFIX)
                .map(str::to_string)
        })
        .map(|failures| {
            let mut deserializer = serde_json::Deserializer::from_str(&failures);
            BTreeMap::deserialize(&mut deserializer).unwrap_or_else(|_| {
                // Read the short-lived predecessor representation emitted by
                // blocked local generations before this map became JSON.
                failures
                    .split("; ")
                    .filter_map(|failure| failure.split_once(": "))
                    .map(|(runtime_id, message)| (runtime_id.to_string(), message.to_string()))
                    .collect()
            })
        })
        .unwrap_or_default()
}

pub(crate) fn record_pending_pair_failure(
    record: &mut super::ManagedAgentRecord,
    key: &ManagedAgentRuntimeKey,
    exit_code: Option<i32>,
    error: &super::storage::AgentLogError,
) {
    let mut failures = pending_pair_failures(record);
    failures.insert(key.runtime_id(), error.message.replace(['\r', '\n'], " "));
    let encoded = serde_json::to_string(&failures).unwrap_or_else(|_| {
        failures
            .iter()
            .map(|(runtime_id, message)| format!("{runtime_id}: {message}"))
            .collect::<Vec<_>>()
            .join("; ")
    });
    let failure_line = format!("{PENDING_PAIR_FAILURE_PREFIX}{encoded}");
    let recovery_lines = record
        .last_error
        .as_deref()
        .into_iter()
        .flat_map(str::lines)
        .filter(|line| line.starts_with(super::UNVERIFIED_JOB_REAP_PREFIX))
        .collect::<Vec<_>>();
    record.last_error = Some(if recovery_lines.is_empty() {
        failure_line
    } else {
        format!("{}\n{failure_line}", recovery_lines.join("\n"))
    });
    record.last_exit_code = exit_code.or(record.last_exit_code);
    record.last_error_code = error.code.or(record.last_error_code);
}

pub(crate) fn finalize_pending_pair_failures(record: &mut super::ManagedAgentRecord) -> bool {
    let failures = pending_pair_failures(record);
    if failures.is_empty() {
        return false;
    }
    record.last_error = Some(
        failures
            .iter()
            .map(|(runtime_id, message)| format!("{runtime_id}: {message}"))
            .collect::<Vec<_>>()
            .join("; "),
    );
    true
}

/// Canonical identity of one managed-agent harness on one relay.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManagedAgentRuntimeKey {
    pub(crate) pubkey: String,
    pub(crate) relay_url: String,
}

impl ManagedAgentRuntimeKey {
    pub(crate) fn new(pubkey: impl Into<String>, relay_url: &str) -> Result<Self, String> {
        let pubkey = pubkey.into();
        if pubkey.len() != 64 || !pubkey.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("managed-agent pubkey must be 64 hexadecimal characters".into());
        }
        if relay_url.len() > MAX_RECOVERY_RELAY_URL_BYTES {
            return Err("managed-agent recovery relay URL exceeds 4096 bytes".into());
        }
        Ok(Self {
            pubkey: pubkey.to_ascii_lowercase(),
            relay_url: buzz_core_pkg::relay::normalize_relay_url(relay_url)
                .map_err(|error| error.to_string())?,
        })
    }

    /// Stable opaque identifier/path suffix derived only from canonical fields.
    pub fn runtime_id(&self) -> String {
        let relay_hash = hex::encode(Sha256::digest(self.relay_url.as_bytes()));
        format!("{}__{relay_hash}", self.pubkey)
    }
}

pub(crate) const MANAGED_AGENT_RECOVERY_STORE_VERSION: u32 = 2;

/// Typed, durable recovery authority for one managed agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManagedAgentRecoveryAuthority {
    generation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) agent_quarantine: Option<ManagedAgentRecoveryEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) uncertain_pairs: Vec<ManagedAgentPairRecoveryEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compatibility_snapshot: Option<ManagedAgentRecoveryCompatibilitySnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManagedAgentRecoveryEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pid: Option<u32>,
    pub(crate) detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManagedAgentPairRecoveryEvidence {
    pub(crate) key: ManagedAgentRuntimeKey,
    pub(crate) pid: u32,
    pub(crate) detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManagedAgentRecoveryStore {
    pub(crate) version: u32,
    #[serde(default, deserialize_with = "deserialize_authorities")]
    pub(crate) authorities: BTreeMap<String, ManagedAgentRecoveryAuthority>,
    #[serde(default)]
    pub(crate) migration_complete: bool,
}

impl Default for ManagedAgentRecoveryStore {
    fn default() -> Self {
        Self {
            version: MANAGED_AGENT_RECOVERY_STORE_VERSION,
            authorities: BTreeMap::new(),
            migration_complete: false,
        }
    }
}

fn deserialize_authorities<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, ManagedAgentRecoveryAuthority>, D::Error>
where
    D: Deserializer<'de>,
{
    struct AuthorityMapVisitor;

    impl<'de> Visitor<'de> for AuthorityMapVisitor {
        type Value = BTreeMap<String, ManagedAgentRecoveryAuthority>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a managed-agent recovery authority map with unique keys")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut authorities = BTreeMap::new();
            while let Some((pubkey, authority)) = map.next_entry()? {
                if authorities.insert(pubkey, authority).is_some() {
                    return Err(serde::de::Error::custom(
                        "duplicate managed-agent recovery authority key",
                    ));
                }
            }
            Ok(authorities)
        }
    }

    deserializer.deserialize_map(AuthorityMapVisitor)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManagedAgentRecoveryCompatibilitySnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runtime_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
}

impl ManagedAgentRecoveryAuthority {
    pub(crate) fn is_empty(&self) -> bool {
        self.agent_quarantine.is_none() && self.uncertain_pairs.is_empty()
    }

    pub(crate) fn from_legacy(runtime_pid: Option<u32>, last_error: Option<&str>) -> Self {
        let legacy_error = last_error.unwrap_or_default();
        let Some(pid) = runtime_pid else {
            if recovery_projection_present(legacy_error) {
                return Self {
                    generation: uuid::Uuid::new_v4().to_string(),
                    agent_quarantine: Some(ManagedAgentRecoveryEvidence {
                        pid: None,
                        detail: format!(
                            "malformed recovery evidence is missing its compatibility PID: {legacy_error}"
                        ),
                    }),
                    uncertain_pairs: Vec::new(),
                    compatibility_snapshot: None,
                };
            }
            return Self::default();
        };
        if recovery_projection_present(legacy_error) {
            let only_known_lines = legacy_error.lines().all(|line| {
                line.starts_with(super::UNVERIFIED_JOB_REAP_PREFIX)
                    || line.starts_with(PENDING_PAIR_FAILURE_PREFIX)
            });
            let parsed: Option<Vec<_>> = legacy_error
                .lines()
                .filter(|line| !line.starts_with(PENDING_PAIR_FAILURE_PREFIX))
                .map(parse_legacy_pair_recovery)
                .collect();
            if let Some(entries) = parsed.filter(|entries| {
                if !only_known_lines {
                    return false;
                }
                if entries.is_empty() {
                    return false;
                }
                let unique = entries
                    .iter()
                    .map(|entry| entry.key.runtime_id())
                    .collect::<std::collections::BTreeSet<_>>();
                unique.len() == entries.len()
            }) {
                let generations = legacy_error
                    .lines()
                    .filter_map(parse_legacy_generation)
                    .collect::<std::collections::BTreeSet<_>>();
                let generation = if generations.len() == 1 {
                    generations.into_iter().next().unwrap_or_default()
                } else {
                    uuid::Uuid::new_v4().to_string()
                };
                return Self {
                    generation,
                    agent_quarantine: None,
                    uncertain_pairs: entries,
                    compatibility_snapshot: None,
                };
            }
        }
        Self {
            generation: uuid::Uuid::new_v4().to_string(),
            agent_quarantine: Some(ManagedAgentRecoveryEvidence {
                pid: Some(pid),
                detail: if legacy_error.is_empty() {
                    "unscoped legacy Windows runtime PID has no owned Child/Job terminal proof"
                        .into()
                } else {
                    format!("unscoped or malformed legacy recovery evidence: {legacy_error}")
                },
            }),
            uncertain_pairs: Vec::new(),
            compatibility_snapshot: None,
        }
    }

    pub(crate) fn admission_error(&self, key: &ManagedAgentRuntimeKey) -> Option<String> {
        if let Some(evidence) = &self.agent_quarantine {
            return Some(format!(
                "managed-agent recovery is quarantined for every relay pair: {}",
                evidence.detail
            ));
        }
        self.uncertain_pairs
            .iter()
            .find(|evidence| evidence.key == *key)
            .map(|evidence| {
                format!(
                    "managed-agent recovery remains uncertain for this exact relay pair (pid {}): {}",
                    evidence.pid, evidence.detail
                )
            })
    }

    pub(crate) fn mark_pair(&mut self, key: &ManagedAgentRuntimeKey, pid: u32, detail: String) {
        self.generation = uuid::Uuid::new_v4().to_string();
        if let Some(existing) = self
            .uncertain_pairs
            .iter_mut()
            .find(|evidence| evidence.key == *key)
        {
            existing.pid = pid;
            existing.detail = detail;
        } else {
            self.uncertain_pairs.push(ManagedAgentPairRecoveryEvidence {
                key: key.clone(),
                pid,
                detail,
            });
        }
        self.uncertain_pairs.sort_by(|left, right| {
            left.key
                .runtime_id()
                .cmp(&right.key.runtime_id())
                .then_with(|| left.pid.cmp(&right.pid))
        });
    }

    pub(crate) fn clear_pair_with_terminal_proof(&mut self, key: &ManagedAgentRuntimeKey) -> bool {
        let old_len = self.uncertain_pairs.len();
        self.uncertain_pairs.retain(|evidence| evidence.key != *key);
        self.uncertain_pairs.len() != old_len
    }

    pub(crate) fn capture_compatibility_snapshot(&mut self, record: &super::ManagedAgentRecord) {
        self.compatibility_snapshot = Some(ManagedAgentRecoveryCompatibilitySnapshot {
            runtime_pid: record.runtime_pid,
            last_error: record.last_error.clone(),
        });
    }

    pub(crate) fn accepts_compatibility_record(&self, record: &super::ManagedAgentRecord) -> bool {
        let has_recovery_projection = record.runtime_pid.is_some()
            || record
                .last_error
                .as_deref()
                .is_some_and(recovery_projection_present);
        if !has_recovery_projection {
            return true;
        }
        self.compatibility_snapshot
            .as_ref()
            .is_some_and(|snapshot| {
                snapshot.runtime_pid == record.runtime_pid
                    && snapshot.last_error == record.last_error
            })
    }

    pub(crate) fn merge_compatibility_record(
        &mut self,
        record: &super::ManagedAgentRecord,
    ) -> Result<(), String> {
        let fallback = Self::from_legacy(record.runtime_pid, record.last_error.as_deref());
        if fallback.is_empty() {
            return Ok(());
        }
        let fallback_generation = fallback.generation.clone();
        if self.agent_quarantine.is_some() || fallback.agent_quarantine.is_some() {
            let details = self
                .agent_quarantine
                .iter()
                .map(|evidence| evidence.detail.as_str())
                .chain(
                    fallback
                        .agent_quarantine
                        .iter()
                        .map(|evidence| evidence.detail.as_str()),
                )
                .chain(
                    self.uncertain_pairs
                        .iter()
                        .map(|evidence| evidence.detail.as_str()),
                )
                .chain(
                    fallback
                        .uncertain_pairs
                        .iter()
                        .map(|evidence| evidence.detail.as_str()),
                )
                .collect::<Vec<_>>()
                .join("; ");
            let pid = self
                .agent_quarantine
                .as_ref()
                .and_then(|evidence| evidence.pid)
                .or_else(|| {
                    fallback
                        .agent_quarantine
                        .as_ref()
                        .and_then(|evidence| evidence.pid)
                })
                .or_else(|| self.uncertain_pairs.first().map(|evidence| evidence.pid))
                .or_else(|| {
                    fallback
                        .uncertain_pairs
                        .first()
                        .map(|evidence| evidence.pid)
                });
            self.agent_quarantine = Some(ManagedAgentRecoveryEvidence {
                pid,
                detail: format!(
                    "conflicting sidecar and compatibility recovery evidence: {details}"
                ),
            });
            self.uncertain_pairs.clear();
        } else {
            for evidence in fallback.uncertain_pairs {
                if let Some(existing) = self
                    .uncertain_pairs
                    .iter_mut()
                    .find(|existing| existing.key == evidence.key)
                {
                    if existing.pid == evidence.pid
                        && fallback_generation == self.generation
                        && evidence
                            .detail
                            .starts_with(TERMINAL_PROOF_PENDING_CLEAR_DETAIL)
                    {
                        existing.detail = evidence.detail;
                    } else if existing.pid != evidence.pid || existing.detail != evidence.detail {
                        self.agent_quarantine = Some(ManagedAgentRecoveryEvidence {
                            pid: Some(existing.pid),
                            detail: format!(
                                "conflicting recovery evidence for exact pair {}",
                                evidence.key.runtime_id()
                            ),
                        });
                        self.uncertain_pairs.clear();
                        break;
                    }
                } else {
                    self.uncertain_pairs.push(evidence);
                }
            }
            self.uncertain_pairs
                .sort_by_key(|evidence| evidence.key.runtime_id());
        }
        self.generation = uuid::Uuid::new_v4().to_string();
        Ok(())
    }

    pub(crate) fn project_compatibility(&self, record: &mut super::ManagedAgentRecord) {
        let pending_failure = pending_pair_failure_line(record.last_error.as_deref());
        let has_pending_failure = pending_failure.is_some();
        if self.is_empty() {
            if self.compatibility_snapshot.is_some() {
                record.runtime_pid = None;
                if record
                    .last_error
                    .as_deref()
                    .is_some_and(recovery_projection_present)
                {
                    record.last_error = pending_failure;
                    if !has_pending_failure {
                        record.last_error_code = None;
                    }
                }
            }
            return;
        }

        record.last_stopped_at = None;
        if let Some(quarantine) = &self.agent_quarantine {
            record.runtime_pid = quarantine.pid.or(record.runtime_pid);
            let recovery = format!(
                "{} generation={} quarantine; {}",
                super::UNVERIFIED_JOB_REAP_PREFIX,
                self.generation,
                quarantine.detail
            );
            record.last_error = Some(match pending_failure {
                Some(failure) => format!("{recovery}\n{failure}"),
                None => recovery,
            });
            if !has_pending_failure {
                record.last_error_code = None;
            }
            return;
        }
        record.runtime_pid = self.uncertain_pairs.first().map(|evidence| evidence.pid);
        let recovery = self
            .uncertain_pairs
            .iter()
            .map(|evidence| {
                let pair = serde_json::to_string(&evidence.key)
                    .unwrap_or_else(|_| evidence.key.runtime_id());
                format!(
                    "{} generation={} pair={} pid={}; {}",
                    super::UNVERIFIED_JOB_REAP_PREFIX,
                    self.generation,
                    pair,
                    evidence.pid,
                    evidence.detail
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        record.last_error = Some(match pending_failure {
            Some(failure) => format!("{recovery}\n{failure}"),
            None => recovery,
        });
        if !has_pending_failure {
            record.last_error_code = None;
        }
    }
}

fn parse_legacy_pair_recovery(line: &str) -> Option<ManagedAgentPairRecoveryEvidence> {
    let body = line.strip_prefix(super::UNVERIFIED_JOB_REAP_PREFIX)?;
    let pair_start = if let Some(generated) = body.strip_prefix(" generation=") {
        let (_, pair) = generated.split_once(" pair=")?;
        pair
    } else {
        body.strip_prefix(" pair=")?
    };
    let pair_end = pair_start.find("} pid=")? + 1;
    let parsed_key: ManagedAgentRuntimeKey = serde_json::from_str(&pair_start[..pair_end]).ok()?;
    let key = ManagedAgentRuntimeKey::new(parsed_key.pubkey.clone(), &parsed_key.relay_url).ok()?;
    if key != parsed_key {
        return None;
    }
    let remainder = pair_start[pair_end..].strip_prefix(" pid=")?;
    let (pid, detail) = remainder.split_once(';')?;
    Some(ManagedAgentPairRecoveryEvidence {
        key,
        pid: pid.parse().ok()?,
        detail: detail.trim().to_string(),
    })
}

fn parse_legacy_generation(line: &str) -> Option<String> {
    let generated = line
        .strip_prefix(super::UNVERIFIED_JOB_REAP_PREFIX)?
        .strip_prefix(" generation=")?;
    let (generation, _) = generated.split_once(' ')?;
    uuid::Uuid::parse_str(generation).ok()?;
    Some(generation.to_string())
}

pub(crate) fn record_terminal_proof_pending_recovery_clear(
    record: &mut super::ManagedAgentRecord,
    key: &ManagedAgentRuntimeKey,
    pid: u32,
    detail: &str,
) {
    let pending_failure = pending_pair_failure_line(record.last_error.as_deref());
    let has_pending_failure = pending_failure.is_some();
    let generation = record
        .last_error
        .as_deref()
        .and_then(|error| error.lines().find_map(parse_legacy_generation))
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let pair = serde_json::to_string(key).unwrap_or_else(|_| key.runtime_id());
    record.runtime_pid = Some(pid);
    record.last_stopped_at = None;
    let recovery = format!(
        "{} generation={generation} pair={pair} pid={pid}; {TERMINAL_PROOF_PENDING_CLEAR_DETAIL}: {detail}",
        super::UNVERIFIED_JOB_REAP_PREFIX
    );
    record.last_error = Some(match pending_failure {
        Some(failure) => format!("{recovery}\n{failure}"),
        None => recovery,
    });
    if !has_pending_failure {
        record.last_error_code = None;
    }
}

pub(crate) fn record_blocked_recovery_admission(_record: &super::ManagedAgentRecord) -> bool {
    // Rejected recovery admission is observational only. Every compatibility
    // field remains byte-for-byte available for an authoritative retry.
    false
}

pub(crate) fn append_shutdown_diagnostic(
    record: &mut super::ManagedAgentRecord,
    detail: &str,
    force_recovery_projection: bool,
) {
    let pending = pending_pair_failure_line(record.last_error.as_deref());
    let mut lines = record
        .last_error
        .as_deref()
        .into_iter()
        .flat_map(str::lines)
        .filter(|line| !line.starts_with(PENDING_PAIR_FAILURE_PREFIX))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if let Some(recovery) = lines
        .iter_mut()
        .find(|line| line.starts_with(super::UNVERIFIED_JOB_REAP_PREFIX))
    {
        recovery.push_str("; ");
        recovery.push_str(detail);
    } else if force_recovery_projection {
        lines.push(format!("{} {detail}", super::UNVERIFIED_JOB_REAP_PREFIX));
    } else {
        // Canonical sidecar authority may be the only recovery evidence. Do
        // not broaden that exact-pair authority into an agent quarantine.
        lines.push(detail.to_string());
    }
    if let Some(pending) = pending {
        lines.push(pending);
    } else {
        record.last_error_code = None;
    }
    record.last_error = Some(lines.join("\n"));
}

pub(crate) fn terminal_proof_pending_recovery_clears(
    record: &super::ManagedAgentRecord,
) -> Vec<ManagedAgentRuntimeKey> {
    ManagedAgentRecoveryAuthority::from_legacy(record.runtime_pid, record.last_error.as_deref())
        .uncertain_pairs
        .into_iter()
        .filter(|evidence| {
            evidence
                .detail
                .starts_with(TERMINAL_PROOF_PENDING_CLEAR_DETAIL)
        })
        .map(|evidence| evidence.key)
        .collect()
}

impl ManagedAgentRecoveryStore {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.version != MANAGED_AGENT_RECOVERY_STORE_VERSION {
            return Err(format!(
                "unsupported managed-agent recovery store version {}",
                self.version
            ));
        }
        if self.authorities.len() > 1024 {
            return Err("managed-agent recovery store exceeds 1024 agents".into());
        }
        if self.authorities.is_empty() && !self.migration_complete {
            return Err(
                "managed-agent recovery store cannot persist unmigrated semantic absence".into(),
            );
        }
        for (pubkey, authority) in &self.authorities {
            if pubkey.len() != 64
                || !pubkey.bytes().all(|byte| byte.is_ascii_hexdigit())
                || *pubkey != pubkey.to_ascii_lowercase()
            {
                return Err(format!(
                    "managed-agent recovery store contains a noncanonical agent key: {pubkey}"
                ));
            }
            if authority.agent_quarantine.is_some() && !authority.uncertain_pairs.is_empty() {
                return Err(format!(
                    "managed-agent recovery store contains contradictory authority for {pubkey}"
                ));
            }
            if uuid::Uuid::parse_str(&authority.generation).is_err() {
                return Err(format!(
                    "managed-agent recovery store contains an invalid generation for {pubkey}"
                ));
            }
            if authority.is_empty() && authority.compatibility_snapshot.is_none() {
                return Err(format!(
                    "managed-agent recovery store contains an empty authority for {pubkey}"
                ));
            }
            if authority.uncertain_pairs.len() > 128 {
                return Err(format!(
                    "managed-agent recovery store exceeds 128 pairs for {pubkey}"
                ));
            }
            if let Some(quarantine) = &authority.agent_quarantine {
                if quarantine.pid == Some(0)
                    || quarantine.detail.trim().is_empty()
                    || quarantine.detail.len() > 4096
                {
                    return Err(format!(
                        "managed-agent recovery store contains invalid quarantine evidence for {pubkey}"
                    ));
                }
            }
            if let Some(snapshot) = &authority.compatibility_snapshot {
                if snapshot.runtime_pid == Some(0)
                    || snapshot
                        .last_error
                        .as_ref()
                        .is_some_and(|error| error.len() > 16_384)
                {
                    return Err(format!(
                        "managed-agent recovery store contains an invalid compatibility snapshot for {pubkey}"
                    ));
                }
                if authority.is_empty()
                    && !snapshot.last_error.as_deref().is_some_and(|error| {
                        error.contains(&format!(" generation={} ", authority.generation))
                    })
                {
                    return Err(format!(
                        "managed-agent recovery tombstone lacks record-backed generation for {pubkey}"
                    ));
                }
            }
            let mut runtime_ids = std::collections::BTreeSet::new();
            for evidence in &authority.uncertain_pairs {
                let canonical = ManagedAgentRuntimeKey::new(
                    evidence.key.pubkey.clone(),
                    &evidence.key.relay_url,
                )?;
                if canonical != evidence.key || evidence.key.pubkey != *pubkey {
                    return Err(format!(
                        "managed-agent recovery store contains a noncanonical or mismatched pair key for {pubkey}"
                    ));
                }
                if evidence.pid == 0
                    || evidence.detail.trim().is_empty()
                    || evidence.detail.len() > 4096
                {
                    return Err(format!(
                        "managed-agent recovery store contains invalid pair evidence for {}",
                        evidence.key.runtime_id()
                    ));
                }
                if !runtime_ids.insert(evidence.key.runtime_id()) {
                    return Err(format!(
                        "managed-agent recovery store contains duplicate authority for pair {}",
                        evidence.key.runtime_id()
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedAgentRuntimeLifecycle {
    Starting,
    Listening,
    Waking,
    Ready,
    Failed,
    Stopped,
}

#[derive(Debug)]
pub struct ManagedAgentPairRuntime {
    pub process: ManagedAgentProcess,
    pub lifecycle: ManagedAgentRuntimeLifecycle,
    pub error: Option<String>,
    /// Unpredictable identity for this exact harness generation. Lifecycle
    /// frames from prior processes are rejected even when the pair is live.
    pub start_nonce: String,
}

impl std::ops::Deref for ManagedAgentPairRuntime {
    type Target = ManagedAgentProcess;

    fn deref(&self) -> &Self::Target {
        &self.process
    }
}

impl std::ops::DerefMut for ManagedAgentPairRuntime {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.process
    }
}

impl ManagedAgentPairRuntime {
    pub fn starting(process: ManagedAgentProcess) -> Self {
        let start_nonce = process.start_nonce.clone();
        Self {
            process,
            lifecycle: ManagedAgentRuntimeLifecycle::Starting,
            error: None,
            start_nonce,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentRuntimeStatus {
    pub pubkey: String,
    pub relay_url: String,
    /// Exact descriptor URL echoed only by reconcile result rows so callers can
    /// correlate a canonical response without normalizing on the frontend.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_relay_url: Option<String>,
    pub local_setup: bool,
    pub lifecycle: ManagedAgentRuntimeLifecycle,
    pub pid: Option<u32>,
    pub error: Option<String>,
    pub log_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentRuntimeLifecycleObserverPayload {
    pub pubkey: String,
    pub relay_url: String,
    pub start_nonce: String,
    pub lifecycle: ManagedAgentRuntimeLifecycle,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentCommunityTarget {
    pub relay_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentRuntimeReceipt {
    pub key: ManagedAgentRuntimeKey,
    pub pid: u32,
    pub desktop_instance_id: String,
    pub started_at: String,
    /// True only when this process was assigned to Buzz's mandatory Windows
    /// kill-on-close Job before its receipt was committed. Missing markers are
    /// legacy and require a separate read-only liveness proof before retirement.
    #[serde(default)]
    pub windows_job_contained: bool,
}

pub(crate) fn process_has_windows_job(process: &ManagedAgentProcess) -> bool {
    #[cfg(windows)]
    {
        process.job.is_some()
    }
    #[cfg(not(windows))]
    {
        let _ = process;
        false
    }
}
