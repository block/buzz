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
//! The receipt is cleared only by the next provider *success* for that agent
//! that the store also records: that deploy superseded the ambiguous one
//! remotely and left a local record of doing so. A later attempt that fails at
//! the provider does neither, however cleanly its `last_error` saves — the
//! earlier deployment may still be running — so it leaves the receipt alone.
//!
//! Receipts hold no secrets: the digest is a hash, and no payload field is
//! copied into them.
//!
//! Both ends of the I/O keep their own no-follow boundary — see
//! [`write_receipt_file`] and [`read_receipt_file`]. A receipt is evidence
//! about a deployment, so neither side may be redirected by a symlink dropped
//! into the directory, and the reader treats every file it finds there as
//! untrusted input with a size and a count bound.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::managed_agents::managed_agents_base_dir;

/// Largest file this reader will accept as a receipt. A receipt is a handful
/// of short strings, so this is generous by orders of magnitude and still
/// bounds what one unreadable file can cost.
const MAX_RECEIPT_BYTES: u64 = 64 * 1024;

/// Most receipt files one read will open. There is one per agent, so this is
/// far above any real store; past it the directory is not this directory.
const MAX_RECEIPT_FILES: usize = 256;

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
/// worse. Callers must have observed a saved provider success first — see the
/// module docs. `remove_file` unlinks the entry itself, so a symlink here
/// costs its link and never its target.
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
    write_receipt_file(&path, &payload)
}

/// Write `payload` to `path` atomically, `0o600`, without following a symlink
/// at `path`.
///
/// Deliberately not [`crate::managed_agents::atomic_write_json_restricted`]:
/// that canonicalizes the final path first, which is right for the agent store
/// — dev worktrees legitimately symlink it into the shared dev data dir — and
/// wrong here. The dev sync shares data files only, never per-run state like
/// `agent-pids/`, so a symlink in this directory is not a configuration, it is
/// a way to make Buzz write receipt contents somewhere of another party's
/// choosing.
///
/// The temp file is created `O_EXCL`, which never follows a link, and renamed
/// over `path`; rename replaces the link entry itself rather than writing
/// through it. Replacing beats rejecting: refusing to write would let anyone
/// who can drop a symlink into this directory suppress the very evidence the
/// receipt exists to preserve.
fn write_receipt_file(path: &Path, payload: &[u8]) -> Result<(), String> {
    use std::io::Write as _;

    let tmp = path.with_extension("json.tmp");
    // A temp file left by an interrupted write is stale by definition, and
    // removing it unlinks that entry (never a link's target) so the O_EXCL
    // create below cannot be blocked forever by one.
    let _ = std::fs::remove_file(&tmp);

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp).map_err(|error| {
        format!(
            "failed to open {} for a deploy receipt: {error}",
            tmp.display()
        )
    })?;
    file.write_all(payload)
        .map_err(|error| format!("failed to write {}: {error}", tmp.display()))?;
    // The bytes must reach the disk before the rename publishes them. A receipt
    // exists to survive the failure that lost the store write — often the same
    // crash or full disk — so a rename that lands ahead of its own contents
    // would leave exactly the empty evidence this file exists to prevent.
    // `atomic_write_json_restricted` got this from `AtomicWriteFile::commit`;
    // doing the write by hand means doing it here.
    file.sync_all()
        .map_err(|error| format!("failed to flush {}: {error}", tmp.display()))?;
    drop(file);

    std::fs::rename(&tmp, path).map_err(|error| {
        let _ = std::fs::remove_file(&tmp);
        format!(
            "failed to move {} onto {}: {error}",
            tmp.display(),
            path.display()
        )
    })
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
        // The directory's own idea of the entry's type, which costs no syscall
        // on whatever a link points at.
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .take(MAX_RECEIPT_FILES)
        .filter_map(|entry| read_receipt_file(&entry.path()))
        .collect();
    receipts.sort_by(|left, right| right.observed_at.cmp(&left.observed_at));
    receipts
}

/// Read one receipt through a single descriptor.
///
/// The open refuses a symlink outright, and every check after it — regular
/// file, size — is made with `fstat` against the descriptor that is then read,
/// so nothing can swap the path between the check and the read. The read is
/// capped as well as the check, so a file that grows in between is truncated
/// rather than followed to whatever it becomes. Anything that fails a check is
/// skipped: this is a best-effort report, and one unreadable file must not
/// hide the rest.
fn read_receipt_file(path: &Path) -> Option<AmbiguousDeployReceipt> {
    use std::io::Read as _;

    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        // Opens a reparse point itself instead of its target, and is ignored
        // for files that are not one — so the `is_file` check below rejects a
        // link rather than reading through it.
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }

    let file = options.open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_RECEIPT_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_RECEIPT_BYTES).read_to_end(&mut bytes).ok()?;
    serde_json::from_slice(&bytes).ok()
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

    /// The write must land on the receipt path itself. A symlink there is
    /// replaced, and the file it pointed at is left byte-for-byte alone — a
    /// receipt write is never a write to a path someone else chose.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_receipt_path_is_replaced_and_its_target_left_unchanged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let elsewhere = tempfile::tempdir().expect("tempdir");
        let target = elsewhere.path().join("victim.json");
        std::fs::write(&target, b"original").expect("seed the symlink target");
        let link = dir.path().join("agent1.json");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");

        write_deploy_receipt_in(dir.path(), &receipt("agent1", "2026-01-01T00:00:00Z"))
            .expect("write");

        assert_eq!(
            std::fs::read(&target).expect("read the symlink target"),
            b"original",
            "the write followed the symlink and overwrote its target"
        );
        assert!(
            !std::fs::symlink_metadata(&link)
                .expect("receipt path")
                .file_type()
                .is_symlink(),
            "the symlink survived the write, so the next one goes through it"
        );
        // And the receipt really was written, at the real path.
        assert_eq!(
            read_deploy_receipts_in(dir.path()),
            vec![receipt("agent1", "2026-01-01T00:00:00Z")]
        );
    }

    /// The reader's side of the same boundary: a symlink is skipped even when
    /// it points at a well-formed receipt, and reading does not touch it.
    #[cfg(unix)]
    #[test]
    fn the_reader_skips_a_symlink_pointing_at_a_valid_receipt() {
        let dir = tempfile::tempdir().expect("tempdir");
        let elsewhere = tempfile::tempdir().expect("tempdir");
        let planted = elsewhere.path().join("planted.json");
        let bytes = serde_json::to_vec(&receipt("agent2", "2026-01-02T00:00:00Z"))
            .expect("serialize a receipt");
        std::fs::write(&planted, &bytes).expect("seed the symlink target");
        let link = dir.path().join("agent2.json");
        std::os::unix::fs::symlink(&planted, &link).expect("symlink");

        assert!(
            read_deploy_receipts_in(dir.path()).is_empty(),
            "the reader followed a symlink out of the receipts directory"
        );
        assert_eq!(
            std::fs::read(&planted).expect("read the symlink target"),
            bytes,
            "the reader changed the symlink target"
        );
    }

    /// Receipts are a few hundred bytes. A file too big to be one is not read
    /// into memory to find that out.
    #[test]
    fn a_file_too_large_to_be_a_receipt_is_skipped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut bloated = receipt("agent1", "2026-01-01T00:00:00Z");
        bloated.save_error = "x".repeat(MAX_RECEIPT_BYTES as usize);
        std::fs::write(
            dir.path().join("agent1.json"),
            serde_json::to_vec(&bloated).expect("serialize"),
        )
        .expect("write");

        assert!(
            read_deploy_receipts_in(dir.path()).is_empty(),
            "an oversized file was read as a receipt"
        );
    }

    /// One read opens a bounded number of files, whatever the directory holds.
    #[test]
    fn the_reader_stops_at_the_file_cap() {
        let dir = tempfile::tempdir().expect("tempdir");
        for index in 0..MAX_RECEIPT_FILES + 8 {
            write_deploy_receipt_in(
                dir.path(),
                &receipt(&format!("agent-{index}"), "2026-01-01T00:00:00Z"),
            )
            .expect("write");
        }

        assert_eq!(read_deploy_receipts_in(dir.path()).len(), MAX_RECEIPT_FILES);
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
