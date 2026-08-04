//! Durable local join between an ACP session and the managed Buzz identity.
//!
//! The receipt deliberately stores no prompt, message, channel, credential,
//! model, or cost content. Codex consumers join `session_id` to the UUID in the
//! rollout filename; `session_meta.id` is only a consistency check because an
//! aborted rollout may not contain a `session_meta` record.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[cfg(test)]
use chrono::DateTime;
#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::io::{BufRead, BufReader};

const SCHEMA_VERSION: u8 = 1;
const MAX_SESSION_ID_BYTES: usize = 256;
const MAX_HARNESS_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionIdentityReceipt {
    pub(crate) schema_version: u8,
    pub(crate) session_id: String,
    pub(crate) agent_pubkey: String,
    pub(crate) harness: String,
    pub(crate) recorded_at: String,
}

#[derive(Debug, Error)]
pub(crate) enum SessionIdentityError {
    #[error("invalid session id")]
    InvalidSessionId,
    #[error("invalid agent pubkey")]
    InvalidAgentPubkey,
    #[error("invalid harness identity")]
    InvalidHarness,
    #[cfg(test)]
    #[error("invalid receipt at line {line}")]
    InvalidReceipt { line: usize },
    #[cfg(test)]
    #[error("conflicting identity for session at line {line}")]
    ConflictingIdentity { line: usize },
    #[error("session identity receipt I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("session identity receipt serialization failed: {0}")]
    Serialize(#[from] serde_json::Error),
}

fn safe_identifier(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes
        && value.chars().all(|character| !character.is_control())
}

fn valid_pubkey(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
fn validate_receipt(receipt: &SessionIdentityReceipt) -> bool {
    receipt.schema_version == SCHEMA_VERSION
        && safe_identifier(&receipt.session_id, MAX_SESSION_ID_BYTES)
        && valid_pubkey(&receipt.agent_pubkey)
        && safe_identifier(&receipt.harness, MAX_HARNESS_BYTES)
        && DateTime::parse_from_rfc3339(&receipt.recorded_at).is_ok()
}

pub(crate) fn append_receipt(
    path: &Path,
    session_id: &str,
    agent_pubkey: &str,
    harness: &str,
) -> Result<(), SessionIdentityError> {
    if !safe_identifier(session_id, MAX_SESSION_ID_BYTES) {
        return Err(SessionIdentityError::InvalidSessionId);
    }
    if !valid_pubkey(agent_pubkey) {
        return Err(SessionIdentityError::InvalidAgentPubkey);
    }
    if !safe_identifier(harness, MAX_HARNESS_BYTES) {
        return Err(SessionIdentityError::InvalidHarness);
    }

    let receipt = SessionIdentityReceipt {
        schema_version: SCHEMA_VERSION,
        session_id: session_id.to_string(),
        agent_pubkey: agent_pubkey.to_string(),
        harness: harness.to_string(),
        recorded_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
    };
    let mut encoded = serde_json::to_vec(&receipt)?;
    encoded.push(b'\n');

    let mut options = OpenOptions::new();
    options.create(true).append(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    restrict_to_owner(&file)?;
    file.write_all(&encoded)?;
    file.sync_data()?;
    Ok(())
}

#[cfg(unix)]
fn restrict_to_owner(file: &File) -> Result<(), std::io::Error> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let metadata = file.metadata()?;
    if metadata.mode() & 0o077 != 0 {
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o600);
        file.set_permissions(permissions)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn restrict_to_owner(_file: &File) -> Result<(), std::io::Error> {
    Ok(())
}

#[cfg(test)]
pub(crate) fn read_receipts(
    path: &Path,
) -> Result<HashMap<String, SessionIdentityReceipt>, SessionIdentityError> {
    let file = File::open(path)?;
    let mut receipts: HashMap<String, SessionIdentityReceipt> = HashMap::new();

    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line_number = index + 1;
        let line = line?;
        let receipt: SessionIdentityReceipt = serde_json::from_str(&line)
            .map_err(|_| SessionIdentityError::InvalidReceipt { line: line_number })?;
        if !validate_receipt(&receipt) {
            return Err(SessionIdentityError::InvalidReceipt { line: line_number });
        }
        if let Some(existing) = receipts.get(&receipt.session_id) {
            if existing.agent_pubkey != receipt.agent_pubkey || existing.harness != receipt.harness
            {
                return Err(SessionIdentityError::ConflictingIdentity { line: line_number });
            }
            continue;
        }
        receipts.insert(receipt.session_id.clone(), receipt);
    }
    Ok(receipts)
}

#[cfg(test)]
mod tests {
    use super::{append_receipt, read_receipts, SessionIdentityReceipt};

    const PUBKEY: &str = "cee956f33a68bd1ace03bb889790b06647f5264a4751604fd2196f574783392e";
    const CODEX_SESSION: &str = "019fcac1-a301-7780-b42c-aebc569b4928";

    #[test]
    fn receipt_round_trips_the_exact_codex_session_and_pubkey() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sessions.jsonl");

        append_receipt(&path, CODEX_SESSION, PUBKEY, "codex-acp")
            .expect("append exact identity receipt");
        let raw: serde_json::Value = serde_json::from_str(
            std::fs::read_to_string(&path)
                .expect("read raw receipt")
                .trim(),
        )
        .expect("parse raw receipt");
        let fields = raw.as_object().expect("receipt object");
        assert_eq!(fields.len(), 5);
        for expected in [
            "schema_version",
            "session_id",
            "agent_pubkey",
            "harness",
            "recorded_at",
        ] {
            assert!(fields.contains_key(expected), "missing {expected}");
        }
        let receipts = read_receipts(&path).expect("read identity receipts");

        assert_eq!(
            receipts.get(CODEX_SESSION),
            Some(&SessionIdentityReceipt {
                schema_version: 1,
                session_id: CODEX_SESSION.to_string(),
                agent_pubkey: PUBKEY.to_string(),
                harness: "codex-acp".to_string(),
                recorded_at: receipts[CODEX_SESSION].recorded_at.clone(),
            })
        );
    }

    #[test]
    fn reopen_preserves_existing_receipts_and_adds_a_new_session() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sessions.jsonl");
        let claude_session = "3e404b95-9c13-4dfa-ac65-6f47da5b2bc6";

        append_receipt(&path, CODEX_SESSION, PUBKEY, "codex-acp").expect("first append");
        append_receipt(&path, claude_session, PUBKEY, "claude-agent-acp")
            .expect("append after reopen");
        let receipts = read_receipts(&path).expect("read both receipts");

        assert_eq!(receipts.len(), 2);
        assert_eq!(receipts[CODEX_SESSION].harness, "codex-acp");
        assert_eq!(receipts[claude_session].harness, "claude-agent-acp");
    }

    #[test]
    fn malformed_jsonl_is_rejected_instead_of_partially_attributed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sessions.jsonl");
        std::fs::write(&path, b"{not-json}\n").expect("write malformed fixture");

        let error = read_receipts(&path).expect_err("malformed receipt must fail closed");

        assert!(error.to_string().contains("line 1"));
    }

    #[test]
    fn unknown_schema_fields_are_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sessions.jsonl");
        let receipt = serde_json::json!({
            "schema_version": 1,
            "session_id": CODEX_SESSION,
            "agent_pubkey": PUBKEY,
            "harness": "codex-acp",
            "recorded_at": "2026-08-04T03:00:00Z",
            "prompt": "must never be accepted",
        });
        std::fs::write(&path, format!("{receipt}\n")).expect("write unknown-field fixture");

        let error = read_receipts(&path).expect_err("unknown fields must fail closed");

        assert!(error.to_string().contains("line 1"));
    }

    #[test]
    fn conflicting_identity_for_one_session_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sessions.jsonl");
        let other_pubkey = "a".repeat(64);
        append_receipt(&path, CODEX_SESSION, PUBKEY, "codex-acp").expect("first mapping");
        append_receipt(&path, CODEX_SESSION, &other_pubkey, "codex-acp")
            .expect("conflicting mapping is persisted for reader validation");

        let error = read_receipts(&path).expect_err("conflict must fail closed");

        assert!(error.to_string().contains("conflicting identity"));
    }

    #[test]
    fn repeated_identical_session_receipts_resolve_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sessions.jsonl");
        append_receipt(&path, CODEX_SESSION, PUBKEY, "codex-acp").expect("first mapping");
        append_receipt(&path, CODEX_SESSION, PUBKEY, "codex-acp").expect("repeated mapping");

        let receipts = read_receipts(&path).expect("identical mapping is idempotent");

        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[CODEX_SESSION].agent_pubkey, PUBKEY);
    }

    #[test]
    fn invalid_identifiers_are_rejected_before_writing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sessions.jsonl");

        let error = append_receipt(&path, "contains\nnewline", PUBKEY, "codex-acp")
            .expect_err("unsafe session id must reject");

        assert!(error.to_string().contains("session id"));
        assert!(!path.exists());
    }

    #[test]
    fn unavailable_parent_path_returns_an_explicit_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("missing-parent").join("sessions.jsonl");

        let error = append_receipt(&path, CODEX_SESSION, PUBKEY, "codex-acp")
            .expect_err("unavailable path must not be reported as persisted");

        assert!(error.to_string().contains("I/O failed"));
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn receipt_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sessions.jsonl");

        append_receipt(&path, CODEX_SESSION, PUBKEY, "codex-acp").expect("append receipt");

        let mode = std::fs::metadata(path)
            .expect("receipt metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
