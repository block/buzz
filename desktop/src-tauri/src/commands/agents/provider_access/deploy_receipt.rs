//! Durable receipts for deploys whose outcome the store did not record.
//!
//! A provider call and the store write that records it are two steps, and the
//! second can fail: the disk fills, the keyring goes away mid-save, the file is
//! replaced under us. When the *first* step succeeded, dropping the error on
//! the floor leaves the worst possible state — a provider that may be running
//! a configuration this app has no record of sending, and no way for a later
//! reader to know.
//!
//! So that case writes a receipt: one `0o600` JSON file per agent under
//! `agents/deploy-receipts/`, written atomically, mirroring the runtime
//! receipts in [`crate::managed_agents::storage`]
//! ([`crate::managed_agents::write_agent_runtime_receipt`]). It records what
//! the provider accepted — the provider id, the returned deployment id, the
//! digest of the fully resolved deploy input, and the record generation it was
//! captured from — plus the store error that lost it. A later reader
//! ([`read_deploy_receipts`]) can then say precisely: the provider may be
//! running the input with this digest, and the store does not reflect it.
//!
//! The receipt is cleared by the next completion for that agent that *does*
//! save, because at that point the store records a known outcome again.
//!
//! Receipts hold no secrets: the digest is a hash, and no payload field is
//! copied into them.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::managed_agents::{atomic_write_json_restricted, managed_agents_base_dir};

/// A deploy the provider accepted and the store failed to record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::commands::agents) struct AmbiguousDeployReceipt {
    pub pubkey: String,
    /// Provider the deploy was sent to.
    pub provider_id: String,
    /// The deployment id the provider returned — the value that never reached
    /// the store.
    pub backend_agent_id: String,
    /// Digest of the fully resolved deploy input the provider accepted
    /// (see [`super::payload_digest`]).
    pub payload_digest: String,
    /// The `updated_at` generation the deploy was captured from.
    pub captured_updated_at: String,
    /// Why the store write failed.
    pub save_error: String,
    pub observed_at: String,
}

fn receipts_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = managed_agents_base_dir(app)?.join("deploy-receipts");
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create deploy-receipts dir: {error}"))?;
    Ok(dir)
}

/// Filename for `pubkey`, or an error for one that must not become a path.
/// Rejecting beats sanitizing: a rewritten pubkey could collide with another
/// agent's receipt, and a receipt filed under the wrong agent is worse than no
/// receipt at all.
fn receipt_filename(pubkey: &str) -> Result<String, String> {
    let safe = !pubkey.is_empty()
        && pubkey
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !safe {
        return Err(format!(
            "unsafe agent pubkey for a receipt filename: {pubkey}"
        ));
    }
    Ok(format!("{pubkey}.json"))
}

/// Persist `receipt`, replacing any earlier one for the same agent.
pub(in crate::commands::agents) fn write_deploy_receipt(
    app: &AppHandle,
    receipt: &AmbiguousDeployReceipt,
) -> Result<(), String> {
    write_deploy_receipt_in(&receipts_dir(app)?, receipt)
}

/// Drop the receipt for `pubkey`, if any. Best-effort: a receipt that outlives
/// its ambiguity is noise, but failing a save-succeeded path over it would be
/// worse.
pub(in crate::commands::agents) fn clear_deploy_receipt(app: &AppHandle, pubkey: &str) {
    if let Ok(dir) = receipts_dir(app) {
        clear_deploy_receipt_in(&dir, pubkey);
    }
}

/// Every receipt currently on disk, newest-first by `observed_at`. The reader
/// side of the contract: a non-empty result means at least one provider may be
/// running a configuration the store never recorded.
pub(in crate::commands::agents) fn read_deploy_receipts(
    app: &AppHandle,
) -> Vec<AmbiguousDeployReceipt> {
    match receipts_dir(app) {
        Ok(dir) => read_deploy_receipts_in(&dir),
        Err(_) => Vec::new(),
    }
}

pub(in crate::commands::agents) fn write_deploy_receipt_in(
    dir: &Path,
    receipt: &AmbiguousDeployReceipt,
) -> Result<(), String> {
    let path = dir.join(receipt_filename(&receipt.pubkey)?);
    let payload = serde_json::to_vec(receipt)
        .map_err(|error| format!("failed to serialize deploy receipt: {error}"))?;
    atomic_write_json_restricted(&path, &payload)
}

pub(in crate::commands::agents) fn clear_deploy_receipt_in(dir: &Path, pubkey: &str) {
    if let Ok(name) = receipt_filename(pubkey) {
        let _ = std::fs::remove_file(dir.join(name));
    }
}

pub(in crate::commands::agents) fn read_deploy_receipts_in(
    dir: &Path,
) -> Vec<AmbiguousDeployReceipt> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut receipts: Vec<AmbiguousDeployReceipt> = entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| serde_json::from_slice(&std::fs::read(entry.path()).ok()?).ok())
        .collect();
    receipts.sort_by(|left, right| right.observed_at.cmp(&left.observed_at));
    receipts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt(pubkey: &str, observed_at: &str) -> AmbiguousDeployReceipt {
        AmbiguousDeployReceipt {
            pubkey: pubkey.to_string(),
            provider_id: "kubernetes".to_string(),
            backend_agent_id: "buzz-agent-abc".to_string(),
            payload_digest: "d".repeat(64),
            captured_updated_at: "t1".to_string(),
            save_error: "disk full".to_string(),
            observed_at: observed_at.to_string(),
        }
    }

    /// The whole point of the receipt: a later reader learns what the provider
    /// accepted and that the store does not reflect it.
    #[test]
    fn a_written_receipt_reads_back_intact() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_deploy_receipt_in(dir.path(), &receipt("agent1", "2026-01-01T00:00:00Z"))
            .expect("write");

        let read = read_deploy_receipts_in(dir.path());
        assert_eq!(read, vec![receipt("agent1", "2026-01-01T00:00:00Z")]);
        assert_eq!(read[0].payload_digest.len(), 64);
    }

    #[test]
    fn a_second_receipt_replaces_the_first_for_the_same_agent() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_deploy_receipt_in(dir.path(), &receipt("agent1", "2026-01-01T00:00:00Z"))
            .expect("write");
        let mut newer = receipt("agent1", "2026-01-02T00:00:00Z");
        newer.backend_agent_id = "buzz-agent-def".into();
        write_deploy_receipt_in(dir.path(), &newer).expect("write");

        assert_eq!(read_deploy_receipts_in(dir.path()), vec![newer]);
    }

    #[test]
    fn receipts_are_returned_newest_first_and_cleared_per_agent() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_deploy_receipt_in(dir.path(), &receipt("agent1", "2026-01-01T00:00:00Z"))
            .expect("write");
        write_deploy_receipt_in(dir.path(), &receipt("agent2", "2026-01-03T00:00:00Z"))
            .expect("write");

        let read = read_deploy_receipts_in(dir.path());
        assert_eq!(
            read.iter().map(|r| r.pubkey.as_str()).collect::<Vec<_>>(),
            vec!["agent2", "agent1"]
        );

        clear_deploy_receipt_in(dir.path(), "agent2");
        assert_eq!(
            read_deploy_receipts_in(dir.path())
                .iter()
                .map(|r| r.pubkey.as_str())
                .collect::<Vec<_>>(),
            vec!["agent1"],
            "clearing one agent's receipt removed another's"
        );

        // Clearing an agent with no receipt is a no-op, not an error.
        clear_deploy_receipt_in(dir.path(), "agent3");
        assert_eq!(read_deploy_receipts_in(dir.path()).len(), 1);
    }

    #[test]
    fn a_pubkey_that_could_escape_the_directory_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        for pubkey in ["../escape", "a/b", "", "with space", "dot.dot"] {
            assert!(
                write_deploy_receipt_in(dir.path(), &receipt(pubkey, "t")).is_err(),
                "accepted an unsafe receipt filename: {pubkey}"
            );
        }
        assert!(read_deploy_receipts_in(dir.path()).is_empty());
    }
}
