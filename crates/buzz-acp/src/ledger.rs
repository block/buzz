//! Durable pending-turn ledger — a JSON mirror of the work `EventQueue`
//! still owes a turn for, so a restarted harness can resume without
//! re-prompting.
//!
//! See `PLANS/AGENT_AUTO_RESUME_LEDGER.md` (rev 6.1) for the full design.
//! This module owns the file format, atomic persistence, the per-channel
//! sync contract (with unresolved-record preservation across ordinary
//! rewrites), the unresolved-trigger lifecycle (resolve/invalidate), and
//! the staged-load + single-commit transaction boot uses. Boot
//! orchestration (membership gate, chunked REST fetch, queue wiring) is
//! `lib.rs`'s job — this module only persists and serves what it's told.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::queue::RecoverableTrigger;

/// On-disk format version. Any file with a different value is treated as
/// corrupt (start fresh) rather than partially interpreted.
const LEDGER_VERSION: u32 = 1;

/// Only the first 16 hex chars of the agent pubkey are used for the
/// filename — enough to disambiguate multiple agents sharing a nest
/// without an unwieldy path.
const PUBKEY_PREFIX_LEN: usize = 16;

/// The minimal durable identity of one recoverable event, as persisted.
/// Mirrors [`RecoverableTrigger`] field-for-field; kept as a distinct type
/// so the on-disk format doesn't silently change if the in-memory type
/// grows fields the ledger has no reason to persist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LedgerRecord {
    pub event_id: String,
    pub prompt_tag: String,
    pub admission_seq: u64,
    pub enqueued_at_unix: u64,
    pub cap_exempt: bool,
}

impl From<&RecoverableTrigger> for LedgerRecord {
    fn from(t: &RecoverableTrigger) -> Self {
        LedgerRecord {
            event_id: t.event_id.clone(),
            prompt_tag: t.prompt_tag.clone(),
            admission_seq: t.admission_seq,
            enqueued_at_unix: t.enqueued_at_unix,
            cap_exempt: t.cap_exempt,
        }
    }
}

/// On-disk shape: `{"version":1,"channels":{"<uuid>":[<record>, ...]}}`.
/// No `thread_root`/`created_at` — those are re-fetched with the event.
#[derive(Debug, Serialize, Deserialize)]
struct LedgerFile {
    version: u32,
    channels: HashMap<Uuid, Vec<LedgerRecord>>,
}

/// A read-only snapshot of the ledger file as loaded from disk, staged for
/// boot recovery processing (membership gate, TTL, REST fetch — all owned
/// by `lib.rs`). Nothing here is committed to the runtime `EventQueue`
/// until the caller finishes processing and calls [`Ledger::commit`].
#[derive(Debug, Default)]
pub struct StagedLedger {
    pub channels: HashMap<Uuid, Vec<LedgerRecord>>,
}

/// The durable pending-turn ledger for one agent process.
///
/// `path == None` means the ledger is disabled for this run (state dir
/// couldn't be created) — every operation becomes a silent no-op so the
/// hot path is never blocked by a degraded filesystem.
pub struct Ledger {
    path: Option<PathBuf>,
    /// Per-channel record set reflecting desired in-memory state — always
    /// mutated unconditionally before any persist attempt so removals and
    /// additions are never lost if a write fails. The skip-identical-write
    /// check uses `last_written` for content comparison; `dirty` carries the
    /// "write failed, must retry" signal.
    last_written: HashMap<Uuid, Vec<LedgerRecord>>,
    /// True when the last `persist_candidate` call failed. Forces the next
    /// `sync` to attempt the write even if the record set is unchanged,
    /// ensuring a transient failure is healed at the next opportunity.
    dirty: bool,
    /// Unresolved boot-fetch records, keyed by channel. Preserved across
    /// ordinary `sync()` rewrites of the same channel until resolved
    /// (event arrives live) or invalidated (channel removed) — see
    /// `PLANS/AGENT_AUTO_RESUME_LEDGER.md` §"Unresolved-trigger lifecycle"
    /// (P3-F3).
    unresolved: HashMap<Uuid, Vec<LedgerRecord>>,
}

impl Ledger {
    /// A ledger with no backing file — every operation is a silent no-op.
    /// Used when `--resume-on-restart` is off: no disk I/O at all (an
    /// existing file from a previous run when the flag was on is left
    /// untouched, so toggling the flag back on later still resumes it).
    pub fn disabled() -> Ledger {
        Ledger {
            path: None,
            last_written: HashMap::new(),
            dirty: false,
            unresolved: HashMap::new(),
        }
    }

    /// Load `<state_dir>/pending-turns-<agent_pubkey_prefix16>.json` and
    /// stage its contents for boot processing. Never fails: a missing
    /// file, unparseable JSON, unknown version, or uncreatable state dir
    /// all degrade to an empty stage (one lost recovery at worst, logged),
    /// matching today's every-restart-loses-everything behavior as the
    /// floor.
    ///
    /// `ttl_secs == 0` disables TTL filtering (default); otherwise records
    /// older than `ttl_secs` (by `enqueued_at_unix`) are dropped from the
    /// stage before it's returned — the ledger file on disk is untouched
    /// until the next write, so a filtered-out record isn't destroyed by
    /// merely loading it.
    pub fn load(state_dir: &Path, agent_pubkey_hex: &str, ttl_secs: u64) -> (Ledger, StagedLedger) {
        let empty = || {
            (
                Ledger {
                    path: None,
                    last_written: HashMap::new(),
                    dirty: false,
                    unresolved: HashMap::new(),
                },
                StagedLedger::default(),
            )
        };

        if let Err(e) = std::fs::create_dir_all(state_dir) {
            tracing::warn!(
                dir = %state_dir.display(),
                error = %e,
                "failed to create ACP state dir — resume ledger disabled for this run"
            );
            return empty();
        }

        let prefix: String = agent_pubkey_hex.chars().take(PUBKEY_PREFIX_LEN).collect();
        let path = state_dir.join(format!("pending-turns-{prefix}.json"));

        let channels = match std::fs::read(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "failed to read resume ledger — starting fresh");
                HashMap::new()
            }
            Ok(bytes) => match serde_json::from_slice::<LedgerFile>(&bytes) {
                Ok(file) if file.version == LEDGER_VERSION => file.channels,
                Ok(file) => {
                    tracing::warn!(
                        path = %path.display(),
                        found_version = file.version,
                        expected_version = LEDGER_VERSION,
                        "unsupported resume ledger version — starting fresh (one lost recovery)"
                    );
                    let _ = std::fs::remove_file(&path);
                    HashMap::new()
                }
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "corrupt resume ledger — starting fresh (one lost recovery)"
                    );
                    let _ = std::fs::remove_file(&path);
                    HashMap::new()
                }
            },
        };

        let channels = apply_ttl(channels, ttl_secs);

        (
            Ledger {
                path: Some(path),
                last_written: HashMap::new(),
                dirty: false,
                unresolved: HashMap::new(),
            },
            StagedLedger { channels },
        )
    }

    /// Whether this ledger is actually persisting (state dir creation
    /// succeeded). Boot recovery is a no-op end to end when this is
    /// `false` — there was never anything to stage.
    pub fn is_enabled(&self) -> bool {
        self.path.is_some()
    }

    /// Record an unresolved boot-fetch trigger for `channel_id` — a
    /// ledger entry whose event couldn't be fetched (or validated) during
    /// boot recovery. Kept sorted by `admission_seq` so it merges cleanly
    /// with live triggers in `sync`/`commit`.
    pub fn add_unresolved(&mut self, channel_id: Uuid, record: LedgerRecord) {
        let entries = self.unresolved.entry(channel_id).or_default();
        entries.push(record);
        entries.sort_by_key(|r| r.admission_seq);
    }

    /// Every channel with at least one unresolved record — for boot to
    /// register `EventQueue` ordering barriers over.
    pub fn unresolved_channels(&self) -> Vec<Uuid> {
        self.unresolved.keys().copied().collect()
    }

    /// The set of unresolved `admission_seq`s for `channel_id` — the
    /// input to `EventQueue::set_unresolved_barrier`. Empty if the channel
    /// has no unresolved records.
    pub fn unresolved_seqs(&self, channel_id: Uuid) -> BTreeSet<u64> {
        self.unresolved
            .get(&channel_id)
            .map(|records| records.iter().map(|r| r.admission_seq).collect())
            .unwrap_or_default()
    }

    /// Look up an unresolved record by event id, without consuming it.
    /// Read-only — the caller must call [`Ledger::resolve_unresolved`]
    /// only *after* handing the returned record's event to the queue
    /// (e.g. via `EventQueue::admit_recovered`), preserving the
    /// consume-after-ownership ordering documented there.
    pub fn find_unresolved(&self, channel_id: Uuid, event_id: &str) -> Option<LedgerRecord> {
        self.unresolved
            .get(&channel_id)?
            .iter()
            .find(|r| r.event_id == event_id)
            .cloned()
    }

    /// Consume an unresolved record by event id — call **only** after the
    /// event has been admitted into the queue's ownership (e.g. via
    /// `EventQueue::admit_recovered`), so there is never a window where
    /// the trigger exists in neither the queue nor the ledger's unresolved
    /// set (consume-after-ownership ordering).
    pub fn resolve_unresolved(&mut self, channel_id: Uuid, event_id: &str) {
        if let Some(records) = self.unresolved.get_mut(&channel_id) {
            records.retain(|r| r.event_id != event_id);
            if records.is_empty() {
                self.unresolved.remove(&channel_id);
            }
        }
    }

    /// Purge every durable record — live and unresolved alike — for a
    /// channel that's been removed (membership revoked). Writes
    /// immediately: unlike `sync`, this can't defer to the next dirty-
    /// channel drain, because a removed channel never dirties again.
    pub fn invalidate_channel(&mut self, channel_id: Uuid) {
        let had_unresolved = self.unresolved.remove(&channel_id).is_some();
        let had_written = self.last_written.remove(&channel_id).is_some();
        if had_unresolved || had_written {
            // last_written already reflects intent (channel removed above).
            // Persist; on failure set dirty so the next sync heals the file.
            self.dirty = !self.persist_candidate(&self.last_written.clone());
        }
    }

    /// Persist `channel_id`'s current recoverable trigger set, merged with
    /// any unresolved records for that channel (the two are disjoint by
    /// construction — see the module-level design doc). Skips the write
    /// entirely if the merged record set is unchanged since the last
    /// write to this channel AND no prior write failed (skip-identical-write,
    /// P2-F3; dirty-flag retry, P3-F3).
    pub fn sync(&mut self, channel_id: Uuid, triggers: Vec<RecoverableTrigger>) {
        if self.path.is_none() {
            return;
        }
        let records = self.merge_with_unresolved(channel_id, &triggers);

        let changed = match self.last_written.get(&channel_id) {
            Some(existing) => *existing != records,
            None => !records.is_empty(),
        };
        if !changed && !self.dirty {
            return;
        }
        // Unconditionally update last_written to reflect desired state before
        // persisting — removals must be visible even if the write fails, so
        // subsequent successful writes of any channel naturally omit removed
        // entries and heal the file.
        if records.is_empty() {
            self.last_written.remove(&channel_id);
        } else {
            self.last_written.insert(channel_id, records);
        }
        self.dirty = !self.persist_candidate(&self.last_written.clone());
    }

    /// Boot-only: commit the single transactional snapshot computed as
    /// `live ∪ unresolved` per channel, then resume normal per-mutation
    /// `sync()` behavior. This is the **only** ledger write during boot
    /// recovery (P2-F4) — a crash before this call leaves the pre-crash
    /// file untouched.
    pub fn commit(&mut self, live: HashMap<Uuid, Vec<RecoverableTrigger>>) {
        let mut channels: HashMap<Uuid, Vec<LedgerRecord>> = HashMap::new();
        let all_channels = live.keys().copied().chain(self.unresolved.keys().copied());
        for channel_id in all_channels {
            let triggers = live.get(&channel_id).cloned().unwrap_or_default();
            let records = self.merge_with_unresolved(channel_id, &triggers);
            if !records.is_empty() {
                channels.insert(channel_id, records);
            }
        }
        // Unconditionally update last_written to reflect desired state.
        self.last_written = channels;
        self.dirty = !self.persist_candidate(&self.last_written.clone());
    }

    fn merge_with_unresolved(
        &self,
        channel_id: Uuid,
        triggers: &[RecoverableTrigger],
    ) -> Vec<LedgerRecord> {
        let mut records: Vec<LedgerRecord> = triggers.iter().map(LedgerRecord::from).collect();
        if let Some(unresolved) = self.unresolved.get(&channel_id) {
            records.extend(unresolved.iter().cloned());
            records.sort_by_key(|r| r.admission_seq);
        }
        records
    }

    /// Atomic write: serialize `candidate` to a temp file in the same
    /// directory, then rename over the real path. Returns `true` on success.
    /// A crash mid-write leaves the previous file intact; a crash after the
    /// rename leaves the new one intact — there is no partially-written state.
    /// The caller is responsible for advancing `last_written` only on success
    /// (P3-F3).
    fn persist_candidate(&self, candidate: &HashMap<Uuid, Vec<LedgerRecord>>) -> bool {
        let Some(path) = &self.path else { return true }; // no-path = in-memory only
        let file = LedgerFile {
            version: LEDGER_VERSION,
            channels: candidate.clone(),
        };
        let json = match serde_json::to_vec_pretty(&file) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!(error = %e, "failed to serialize resume ledger — write skipped");
                return false;
            }
        };
        let mut tmp_name = path.as_os_str().to_os_string();
        tmp_name.push(".tmp");
        let tmp_path = PathBuf::from(tmp_name);
        if let Err(e) = std::fs::write(&tmp_path, &json) {
            tracing::error!(path = %tmp_path.display(), error = %e, "failed to write resume ledger temp file");
            return false;
        }
        if let Err(e) = std::fs::rename(&tmp_path, path) {
            tracing::error!(path = %path.display(), error = %e, "failed to rename resume ledger into place");
            return false;
        }
        true
    }
}

/// Drop records older than `ttl_secs` (by `enqueued_at_unix`), dropping
/// the whole channel entry if nothing survives. `ttl_secs == 0` disables
/// filtering entirely — the default, since the ledger only ever holds
/// never-completed turns and a long-idle relaunch must still resume them.
fn apply_ttl(
    channels: HashMap<Uuid, Vec<LedgerRecord>>,
    ttl_secs: u64,
) -> HashMap<Uuid, Vec<LedgerRecord>> {
    if ttl_secs == 0 {
        return channels;
    }
    let now = crate::relay::unix_now_secs();
    channels
        .into_iter()
        .filter_map(|(channel_id, records)| {
            let kept: Vec<LedgerRecord> = records
                .into_iter()
                .filter(|r| now.saturating_sub(r.enqueued_at_unix) <= ttl_secs)
                .collect();
            if kept.is_empty() {
                None
            } else {
                Some((channel_id, kept))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(event_id: &str, seq: u64, enqueued_at_unix: u64, cap_exempt: bool) -> LedgerRecord {
        LedgerRecord {
            event_id: event_id.to_string(),
            prompt_tag: "mention".to_string(),
            admission_seq: seq,
            enqueued_at_unix,
            cap_exempt,
        }
    }

    fn trigger(
        event_id: &str,
        seq: u64,
        enqueued_at_unix: u64,
        cap_exempt: bool,
    ) -> RecoverableTrigger {
        RecoverableTrigger {
            event_id: event_id.to_string(),
            prompt_tag: "mention".to_string(),
            admission_seq: seq,
            enqueued_at_unix,
            cap_exempt,
        }
    }

    fn read_file_channels(path: &Path) -> HashMap<Uuid, Vec<LedgerRecord>> {
        let bytes = std::fs::read(path).expect("ledger file should exist");
        let file: LedgerFile = serde_json::from_slice(&bytes).expect("valid ledger JSON");
        assert_eq!(file.version, LEDGER_VERSION);
        file.channels
    }

    #[test]
    fn test_load_missing_file_returns_empty_staged_and_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let (ledger, staged) = Ledger::load(dir.path(), "deadbeefdeadbeefdeadbeef", 0);
        assert!(ledger.is_enabled());
        assert!(staged.channels.is_empty());
    }

    #[test]
    fn test_load_corrupt_file_starts_fresh_and_deletes_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending-turns-deadbeefdeadbeef.json");
        std::fs::write(&path, b"not json").unwrap();

        let (ledger, staged) = Ledger::load(dir.path(), "deadbeefdeadbeefdeadbeef", 0);
        assert!(ledger.is_enabled());
        assert!(staged.channels.is_empty());
        assert!(!path.exists(), "corrupt file should be deleted");
    }

    #[test]
    fn test_load_unknown_version_starts_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending-turns-deadbeefdeadbeef.json");
        std::fs::write(&path, br#"{"version":99,"channels":{}}"#).unwrap();

        let (_ledger, staged) = Ledger::load(dir.path(), "deadbeefdeadbeefdeadbeef", 0);
        assert!(staged.channels.is_empty());
        assert!(!path.exists());
    }

    #[test]
    fn test_load_disabled_when_state_dir_uncreatable() {
        let dir = tempfile::tempdir().unwrap();
        // A regular file blocking the directory path forces create_dir_all
        // to fail.
        let blocked = dir.path().join("state_as_file");
        std::fs::write(&blocked, b"not a dir").unwrap();

        let (ledger, staged) = Ledger::load(&blocked, "deadbeefdeadbeefdeadbeef", 0);
        assert!(!ledger.is_enabled());
        assert!(staged.channels.is_empty());
    }

    #[test]
    fn test_commit_writes_merged_snapshot_sorted_by_admission_seq() {
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _staged) = Ledger::load(dir.path(), "deadbeefdeadbeefdeadbeef", 0);
        let ch = Uuid::new_v4();
        ledger.add_unresolved(ch, record("unresolved-a", 1, 100, false));

        let mut live = HashMap::new();
        live.insert(ch, vec![trigger("live-b", 2, 200, false)]);
        ledger.commit(live);

        let on_disk = read_file_channels(ledger.path.as_ref().unwrap());
        let recs = on_disk.get(&ch).expect("channel present");
        assert_eq!(
            recs.iter().map(|r| r.event_id.as_str()).collect::<Vec<_>>(),
            vec!["unresolved-a", "live-b"]
        );
    }

    #[test]
    fn test_sync_skips_identical_write() {
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _staged) = Ledger::load(dir.path(), "deadbeefdeadbeefdeadbeef", 0);
        let ch = Uuid::new_v4();
        let path = ledger.path.clone().unwrap();

        ledger.sync(ch, vec![trigger("a", 1, 100, false)]);
        assert!(path.exists());

        // Simulate an external observer noticing the file, then remove it —
        // an identical follow-up sync must NOT recreate it.
        std::fs::remove_file(&path).unwrap();
        ledger.sync(ch, vec![trigger("a", 1, 100, false)]);
        assert!(!path.exists(), "identical sync should not rewrite the file");

        // A genuinely different trigger set must write.
        ledger.sync(
            ch,
            vec![trigger("a", 1, 100, false), trigger("b", 2, 200, false)],
        );
        assert!(path.exists(), "changed sync must rewrite the file");
    }

    #[test]
    fn test_sync_preserves_unresolved_entries_across_channel_rewrite() {
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _staged) = Ledger::load(dir.path(), "deadbeefdeadbeefdeadbeef", 0);
        let ch = Uuid::new_v4();
        ledger.add_unresolved(ch, record("unresolved-a", 1, 100, false));

        // Unrelated live work on the same channel completes and the queue
        // reports an empty recoverable set — mark_complete's sync must not
        // erase the only durable proof of the unfetched trigger.
        ledger.sync(ch, vec![]);

        let on_disk = read_file_channels(ledger.path.as_ref().unwrap());
        let recs = on_disk.get(&ch).expect("unresolved record must survive");
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].event_id, "unresolved-a");
    }

    #[test]
    fn test_resolve_unresolved_removes_record_then_sync_omits_it() {
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _staged) = Ledger::load(dir.path(), "deadbeefdeadbeefdeadbeef", 0);
        let ch = Uuid::new_v4();
        ledger.add_unresolved(ch, record("unresolved-a", 1, 100, false));
        assert_eq!(ledger.unresolved_seqs(ch), BTreeSet::from([1]));

        ledger.resolve_unresolved(ch, "unresolved-a");
        assert!(ledger.unresolved_seqs(ch).is_empty());

        // Now admitted live (admit_recovered would push it back through the
        // queue) — the next sync carries it as an ordinary trigger, and the
        // ledger must not duplicate it via the (now-empty) unresolved table.
        ledger.sync(ch, vec![trigger("unresolved-a", 1, 100, false)]);
        let on_disk = read_file_channels(ledger.path.as_ref().unwrap());
        assert_eq!(on_disk.get(&ch).unwrap().len(), 1);
    }

    #[test]
    fn test_invalidate_channel_purges_unresolved_and_last_written() {
        let dir = tempfile::tempdir().unwrap();
        let (mut ledger, _staged) = Ledger::load(dir.path(), "deadbeefdeadbeefdeadbeef", 0);
        let ch = Uuid::new_v4();
        ledger.add_unresolved(ch, record("unresolved-a", 1, 100, false));
        ledger.sync(ch, vec![trigger("live-b", 2, 200, false)]);
        assert!(!ledger.unresolved_seqs(ch).is_empty());

        ledger.invalidate_channel(ch);

        assert!(ledger.unresolved_seqs(ch).is_empty());
        let on_disk = read_file_channels(ledger.path.as_ref().unwrap());
        assert!(
            !on_disk.contains_key(&ch),
            "invalidated channel must not resurrect on re-add"
        );
    }

    #[test]
    fn test_ttl_filter_default_zero_keeps_all_records() {
        let dir = tempfile::tempdir().unwrap();
        let ch = Uuid::new_v4();
        let path = dir.path().join("pending-turns-deadbeefdeadbeef.json");
        let mut channels = HashMap::new();
        channels.insert(ch, vec![record("old", 1, 0, false)]);
        std::fs::write(
            &path,
            serde_json::to_vec(&LedgerFile {
                version: 1,
                channels,
            })
            .unwrap(),
        )
        .unwrap();

        let (_ledger, staged) = Ledger::load(dir.path(), "deadbeefdeadbeefdeadbeef", 0);
        assert_eq!(staged.channels.get(&ch).unwrap().len(), 1);
    }

    #[test]
    fn test_ttl_filter_drops_expired_records() {
        let dir = tempfile::tempdir().unwrap();
        let ch = Uuid::new_v4();
        let path = dir.path().join("pending-turns-deadbeefdeadbeef.json");
        let mut channels = HashMap::new();
        channels.insert(ch, vec![record("ancient", 1, 0, false)]);
        std::fs::write(
            &path,
            serde_json::to_vec(&LedgerFile {
                version: 1,
                channels,
            })
            .unwrap(),
        )
        .unwrap();

        // enqueued_at_unix = 0 (1970) is far older than any positive TTL.
        let (_ledger, staged) = Ledger::load(dir.path(), "deadbeefdeadbeefdeadbeef", 60);
        assert!(
            !staged.channels.contains_key(&ch),
            "expired-only channel should be dropped entirely"
        );
    }

    #[test]
    fn test_load_round_trips_cap_exempt_for_promotion_rule() {
        let dir = tempfile::tempdir().unwrap();
        let ch_no_exempt = Uuid::new_v4();
        let ch_one_exempt = Uuid::new_v4();
        let path = dir.path().join("pending-turns-deadbeefdeadbeef.json");
        let mut channels = HashMap::new();
        channels.insert(
            ch_no_exempt,
            vec![record("a", 1, 100, false), record("b", 2, 100, false)],
        );
        channels.insert(
            ch_one_exempt,
            vec![record("c", 1, 100, true), record("d", 2, 100, false)],
        );
        std::fs::write(
            &path,
            serde_json::to_vec(&LedgerFile {
                version: 1,
                channels,
            })
            .unwrap(),
        )
        .unwrap();

        let (_ledger, staged) = Ledger::load(dir.path(), "deadbeefdeadbeefdeadbeef", 0);
        // Round-trip must preserve the exact persisted cap_exempt bits —
        // `boot_recover`'s whole-snapshot promotion rule reads these values
        // directly from the staged ledger before deciding per-channel promotion.
        assert!(staged.channels[&ch_no_exempt].iter().all(|r| !r.cap_exempt));
        let one_exempt = &staged.channels[&ch_one_exempt];
        assert!(one_exempt.iter().any(|r| r.cap_exempt));
        assert!(one_exempt.iter().any(|r| !r.cap_exempt));
    }
}
