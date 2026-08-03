use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::ManagedAgentProcess;

/// Canonical identity of one managed-agent harness on one relay.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentRuntimeKey {
    pub pubkey: String,
    pub relay_url: String,
}

impl ManagedAgentRuntimeKey {
    pub fn new(pubkey: impl Into<String>, relay_url: &str) -> Result<Self, String> {
        let pubkey = pubkey.into();
        if pubkey.len() != 64 || !pubkey.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("managed-agent pubkey must be 64 hexadecimal characters".into());
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedAgentRuntimeLifecycle {
    Starting,
    Listening,
    Ready,
    #[serde(alias = "waking")]
    Recovering,
    LegacyRuntimeActive,
    ManualLegacyStopRequired,
    Failed,
    Stopped,
}

#[derive(Debug)]
pub struct ManagedAgentPairRuntime {
    /// Launch handle exists only in the Desktop instance that spawned this
    /// generation. It is never lifecycle authority and may be absent after
    /// authenticated adoption.
    pub process: Option<ManagedAgentProcess>,
    pub receipt: Option<buzz_runtime_pkg::protocol::RuntimeReceipt>,
    pub legacy_receipt: Option<LegacyManagedAgentRuntimeReceipt>,
    pub receipt_path: PathBuf,
    pub controller: Option<buzz_runtime_pkg::client::RuntimeClient>,
    pub observer_nonce: Option<String>,
    pub active_jobs: Vec<uuid::Uuid>,
    pub active_assignment: Option<buzz_runtime_pkg::protocol::AssignmentStatusSnapshot>,
    pub active_job: Option<buzz_runtime_pkg::protocol::JobStatus>,
    pub lifecycle: ManagedAgentRuntimeLifecycle,
    pub error: Option<String>,
}

impl ManagedAgentPairRuntime {
    pub fn connected(
        process: Option<ManagedAgentProcess>,
        receipt: buzz_runtime_pkg::protocol::RuntimeReceipt,
        receipt_path: PathBuf,
        controller: buzz_runtime_pkg::client::RuntimeClient,
        status: &buzz_runtime_pkg::protocol::RuntimeStatus,
        active_job: Option<buzz_runtime_pkg::protocol::JobStatus>,
    ) -> Self {
        let observer_nonce = process.as_ref().map(|process| process.start_nonce.clone());
        Self {
            process,
            receipt: Some(receipt),
            legacy_receipt: None,
            receipt_path,
            controller: Some(controller),
            observer_nonce,
            active_jobs: status.active_jobs.clone(),
            active_assignment: status.active_assignment.clone(),
            active_job,
            lifecycle: if status.recovering {
                ManagedAgentRuntimeLifecycle::Recovering
            } else {
                ManagedAgentRuntimeLifecycle::Ready
            },
            error: None,
        }
    }

    pub fn legacy(
        process: ManagedAgentProcess,
        receipt: LegacyManagedAgentRuntimeReceipt,
        receipt_path: PathBuf,
    ) -> Self {
        let observer_nonce = Some(process.start_nonce.clone());
        Self {
            process: Some(process),
            receipt: None,
            legacy_receipt: Some(receipt),
            receipt_path,
            controller: None,
            observer_nonce,
            active_jobs: Vec::new(),
            active_assignment: None,
            active_job: None,
            lifecycle: ManagedAgentRuntimeLifecycle::Ready,
            error: None,
        }
    }

    pub fn apply_authenticated_status(
        &mut self,
        status: &buzz_runtime_pkg::protocol::RuntimeStatus,
        active_job: Option<buzz_runtime_pkg::protocol::JobStatus>,
    ) {
        self.lifecycle = if status.recovering {
            ManagedAgentRuntimeLifecycle::Recovering
        } else {
            ManagedAgentRuntimeLifecycle::Ready
        };
        self.active_jobs.clone_from(&status.active_jobs);
        self.active_assignment.clone_from(&status.active_assignment);
        self.active_job = active_job;
        self.error = None;
    }

    pub fn pid(&self) -> u32 {
        self.receipt
            .as_ref()
            .map(|receipt| receipt.pid)
            .or_else(|| self.legacy_receipt.as_ref().map(|receipt| receipt.pid))
            .expect("managed runtime has a receipt")
    }

    pub fn is_legacy(&self) -> bool {
        self.legacy_receipt.is_some()
    }

    pub fn log_path(&self) -> Option<&Path> {
        self.process
            .as_ref()
            .map(|process| process.log_path.as_path())
    }
    pub fn setup_mode(&self) -> bool {
        self.process
            .as_ref()
            .is_some_and(|process| process.setup_mode)
    }

    pub fn adapter_availability(&self) -> Option<&super::AcpAvailabilityStatus> {
        self.process
            .as_ref()
            .and_then(|process| process.adapter_availability.as_ref())
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_assignment: Option<buzz_runtime_pkg::protocol::AssignmentStatusSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_job: Option<buzz_runtime_pkg::protocol::JobStatus>,
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

pub(crate) const RUNTIME_LOCK_PROTOCOL_VERSION: u8 = 1;

pub(crate) fn runtime_lock_path_hash(lock_path: &Path) -> String {
    hex::encode(Sha256::digest(lock_path.as_os_str().as_encoded_bytes()))
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyManagedAgentRuntimeReceipt {
    #[serde(default = "legacy_schema_version")]
    pub schema_version: u8,
    pub key: ManagedAgentRuntimeKey,
    pub pid: u32,
    /// Anti-PID-reuse marker written by the lock-owning schema-v1 harness.
    #[serde(default)]
    pub process_start_marker: String,
    pub desktop_instance_id: String,
    pub started_at: String,
    /// Phase-0 lock proof. Zero means a pre-lock schema-v1 receipt.
    #[serde(default)]
    pub lock_protocol_version: u8,
    /// SHA-256 of the exact lock path passed to `buzz-acp`.
    #[serde(default)]
    pub lock_path_hash: String,
}

fn legacy_schema_version() -> u8 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_lock_schema_v1_receipt_remains_deserializable() {
        let receipt: LegacyManagedAgentRuntimeReceipt = serde_json::from_value(serde_json::json!({
            "key": {
                "pubkey": "aa".repeat(32),
                "relayUrl": "wss://relay.example"
            },
            "pid": 42,
            "desktopInstanceId": "legacy-desktop",
            "startedAt": "2026-08-01T00:00:00Z"
        }))
        .expect("deserialize pre-lock receipt");
        assert_eq!(receipt.lock_protocol_version, 0);
        assert!(receipt.lock_path_hash.is_empty());
        assert!(receipt.process_start_marker.is_empty());
    }

    #[test]
    fn authenticated_status_retains_assignment_and_active_job_detail() {
        use buzz_runtime_pkg::protocol::{
            AssignmentState, AssignmentStatusSnapshot, JobState, JobStatus, PublicationState,
            RuntimeDiagnostics, RuntimeStatus, SecretToken, WorkState, CONTROL_PROTOCOL_VERSION,
            RUNTIME_RECEIPT_SCHEMA_VERSION,
        };
        use chrono::Utc;
        use uuid::Uuid;

        let generation = Uuid::new_v4();
        let job_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();
        let now = Utc::now();
        let assignment = AssignmentStatusSnapshot {
            assignment_id: "assignment-1".into(),
            source_event_id: Some("source-event".into()),
            channel_id,
            state: AssignmentState::Blocked,
            summary: "Repair JAC-575".into(),
            active_job_id: Some(job_id),
            last_progress_at: now,
            has_blocker: true,
        };
        let job = JobStatus {
            job_id,
            request_event_id: Some("request-event".into()),
            source_event_id: Some("source-event".into()),
            channel_id,
            state: JobState::Running,
            attempt: 1,
            progress_seq: 7,
            summary: "Receipt verification".into(),
            started_at: Some(now),
            finished_at: None,
            exit_code: None,
            error_code: None,
            publication_state: PublicationState::Failed,
            runner_pid: Some(77),
            runner_start_marker: Some("marker".into()),
        };
        let control_status = RuntimeStatus {
            runtime_id: "runtime-1".into(),
            generation,
            work_state: WorkState::Blocked,
            recovering: false,
            recovery_reason: None,
            queued_inbox: 0,
            in_turn_inbox: 0,
            dead_letter_inbox: 0,
            capacity_rejections: 0,
            active_assignment: Some(assignment.clone()),
            active_job: Some(job_id),
            active_jobs: vec![job_id],
            diagnostics: RuntimeDiagnostics::default(),
        };
        let receipt = buzz_runtime_pkg::protocol::RuntimeReceipt {
            schema_version: RUNTIME_RECEIPT_SCHEMA_VERSION,
            key: buzz_runtime_pkg::protocol::ManagedAgentRuntimeKey {
                pubkey: "aa".repeat(32),
                relay_url: "wss://relay.example".into(),
            },
            runtime_id: "runtime-1".into(),
            pid: 42,
            process_start_marker: "marker".into(),
            generation,
            control_addr: "127.0.0.1:12345".parse().unwrap(),
            controller_token: SecretToken::new("11".repeat(32)),
            model_token: SecretToken::new("22".repeat(32)),
            started_at: now,
            protocol_version: CONTROL_PROTOCOL_VERSION,
            lock_protocol_version: 1,
            lock_path_hash: "33".repeat(32),
            ready: true,
        };
        let mut runtime = ManagedAgentPairRuntime {
            process: None,
            receipt: Some(receipt),
            legacy_receipt: None,
            receipt_path: PathBuf::from("receipt.json"),
            controller: None,
            observer_nonce: None,
            active_jobs: vec![],
            active_assignment: None,
            active_job: None,
            lifecycle: ManagedAgentRuntimeLifecycle::Starting,
            error: Some("stale".into()),
        };

        runtime.apply_authenticated_status(&control_status, Some(job.clone()));

        assert_eq!(runtime.active_assignment, Some(assignment.clone()));
        assert_eq!(runtime.active_job, Some(job.clone()));
        assert_eq!(runtime.active_jobs, vec![job_id]);
        let dto = ManagedAgentRuntimeStatus {
            pubkey: "aa".repeat(32),
            relay_url: "wss://relay.example".into(),
            requested_relay_url: None,
            local_setup: true,
            lifecycle: runtime.lifecycle,
            pid: Some(42),
            error: runtime.error,
            log_path: None,
            active_assignment: runtime.active_assignment,
            active_job: runtime.active_job,
        };
        let json = serde_json::to_value(dto).unwrap();
        assert_eq!(json["activeAssignment"]["sourceEventId"], "source-event");
        assert_eq!(json["activeAssignment"]["hasBlocker"], true);
        assert_eq!(json["activeJob"]["progressSeq"], 7);
        assert_eq!(json["activeJob"]["publicationState"], "failed");
    }

    #[test]
    fn phase_zero_receipt_serializes_lock_and_process_proof() {
        let receipt = LegacyManagedAgentRuntimeReceipt {
            schema_version: 1,
            key: ManagedAgentRuntimeKey::new("aa".repeat(32), "wss://relay.example")
                .expect("runtime key"),
            pid: 42,
            process_start_marker: "pid-start-marker".into(),
            desktop_instance_id: "desktop-instance".into(),
            started_at: "2026-08-01T00:00:00Z".into(),
            lock_protocol_version: RUNTIME_LOCK_PROTOCOL_VERSION,
            lock_path_hash: "ab".repeat(32),
        };
        let value = serde_json::to_value(receipt).expect("serialize receipt");

        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["processStartMarker"], "pid-start-marker");
        assert_eq!(value["lockProtocolVersion"], 1);
        assert_eq!(value["lockPathHash"], "ab".repeat(32));
    }
}
