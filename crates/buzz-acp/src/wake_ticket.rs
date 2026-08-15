//! Wake-ticket persistence — today-slice.
//!
//! `EventQueue` (see [`crate::queue`]) is RAM-only: a mention that is queued
//! but not yet consumed does not survive process death. This module gives
//! those in-flight mentions a durable record on disk so a bounced `buzz-acp`
//! process can pick the work back up, even though the connect-time watermark
//! (`lib.rs`) would otherwise treat it as pre-history.
//!
//! Scope is deliberately narrow — see `PLANS/WAKE_TICKET_SPEC.md` §Gate
//! (Oksana, 2026-08-15) for the binding contract this module implements:
//!
//! - One ticket per mention `event_id`, keyed by that id.
//! - `open` → `claimed` → `done` (compacted away) or `drop` (kept for audit).
//! - `mark_complete` is a lock release, not a completion signal — callers in
//!   `lib.rs` decide `done` from `PromptOutcome::Ok` plus "no batch to retry".
//! - Persistence is best-effort after construction: a write failure is
//!   logged and swallowed so a ticket-store hiccup never fails a live turn.
//!   Only [`WakeTicketStore::open`] is fallible — an unavailable or
//!   already-locked directory must stop the process before it double-runs.
//!
//! This module has no knowledge of the relay, the author gate, or channel
//! membership — boot-time replay validation lives in `lib.rs`, which already
//! owns those checks for the live event path.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Maximum open+claimed tickets retained per channel. Mirrors
/// `queue::MAX_PENDING_PER_CHANNEL` — a ticket is only ever written for an
/// event the in-memory queue already accepted, so this is a belt-and-suspenders
/// cap, not the primary backpressure mechanism.
const MAX_PENDING_PER_CHANNEL: usize = 500;

const TICKETS_FILE: &str = "wake-tickets.jsonl";
const LOCK_FILE: &str = ".wake-tickets.lock";

/// Lifecycle state of a wake ticket. See `PLANS/WAKE_TICKET_SPEC.md` §Lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketState {
    /// Accepted by `EventQueue::push`; not yet handed to an agent.
    Open,
    /// Drained into a `FlushBatch` an agent is actively working.
    Claimed,
    /// Turn completed successfully and nothing was requeued. Compacted away
    /// on the next [`WakeTicketStore::open`].
    Done,
    /// Dead-lettered, auth-failed, or the channel was removed. Kept on disk
    /// for audit; never replayed.
    Drop,
}

/// One durable record of an unconsumed (or recently consumed) mention.
///
/// `event` is the signed `nostr::Event` as accepted — the mention itself,
/// not an assembled LLM prompt and not an ACP transcript. No private keys.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticket {
    pub event_id: String,
    pub channel_id: Uuid,
    /// Event's own `created_at`, not local clock.
    pub created_at: u64,
    /// The mentioned agent (this unit).
    pub pubkey: String,
    /// When this unit first queued the event.
    pub seen_at: u64,
    pub state: TicketState,
    pub event: nostr::Event,
    /// The rule tag that matched this event in `filter::match_event` at
    /// write time (e.g. `"@mention"`). Boot replay reconstructs a
    /// `QueuedEvent` straight from the ticket without re-running rule
    /// matching — subscription rules aren't available that early (channel
    /// discovery hasn't happened, there's no live relay yet) and rules may
    /// have changed since the ticket was written anyway. Captured, not
    /// derived, so replay reproduces what was actually queued.
    pub prompt_tag: String,
}

struct Inner {
    file: File,
    /// Last-write-wins mirror of every line ever appended this run, minus
    /// what boot compaction already dropped. Keyed by `event_id`.
    index: HashMap<String, Ticket>,
}

/// Durable, single-writer ticket store for one agent's wake-ticket directory.
///
/// Holds an exclusive `flock` on a lock file inside `dir` for the process
/// lifetime. A second process opening the same directory fails immediately —
/// dual-run of the same identity against the same ticket dir is a crash, not
/// a race, so [`WakeTicketStore::open`] never blocks waiting for the lock.
pub struct WakeTicketStore {
    dir: PathBuf,
    // Held for the process lifetime; never read after construction. Dropping
    // it releases the flock, so it must outlive every other use of `dir`.
    _lock_file: File,
    inner: Mutex<Inner>,
}

#[cfg(unix)]
fn chmod_600(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn chmod_600(_path: &Path) -> io::Result<()> {
    Ok(())
}

impl WakeTicketStore {
    /// Open (creating if needed) the ticket store rooted at `dir`.
    ///
    /// Acquires the exclusive flock, loads any existing `wake-tickets.jsonl`,
    /// collapses it last-write-wins by `event_id`, drops `done` entries, and
    /// rewrites the compacted result via tmp+rename before returning. Fails
    /// if the lock is already held or any filesystem step fails — callers
    /// must treat that as fatal, per the gate ("Lock fail → exit").
    pub fn open(dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        #[cfg(unix)]
        chmod_dir_700(dir)?;

        let lock_path = dir.join(LOCK_FILE);
        let lock_file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)?;
        lock_file.try_lock_exclusive().map_err(|_| {
            io::Error::other(format!(
                "wake-ticket dir {} is already locked by another process \
                 — dual-run is a hard stop",
                dir.display()
            ))
        })?;

        let existing = Self::load_raw(dir)?;
        let survivors: Vec<Ticket> = existing
            .into_iter()
            .filter(|t| t.state != TicketState::Done)
            .collect();
        Self::write_compacted(dir, &survivors)?;

        let tickets_path = dir.join(TICKETS_FILE);
        let file = OpenOptions::new().append(true).open(&tickets_path)?;

        let index = survivors
            .into_iter()
            .map(|t| (t.event_id.clone(), t))
            .collect();

        Ok(Self {
            dir: dir.to_path_buf(),
            _lock_file: lock_file,
            inner: Mutex::new(Inner { file, index }),
        })
    }

    /// Load `wake-tickets.jsonl` (if present) and collapse to last-write-wins
    /// per `event_id`, preserving first-seen order. A line that fails to
    /// parse is logged and skipped rather than failing the whole load — a
    /// single corrupt line must not block boot.
    fn load_raw(dir: &Path) -> io::Result<Vec<Ticket>> {
        let path = dir.join(TICKETS_FILE);
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut order: Vec<String> = Vec::new();
        let mut by_id: HashMap<String, Ticket> = HashMap::new();
        for (lineno, line) in BufReader::new(file).lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Ticket>(&line) {
                Ok(ticket) => {
                    if !by_id.contains_key(&ticket.event_id) {
                        order.push(ticket.event_id.clone());
                    }
                    by_id.insert(ticket.event_id.clone(), ticket);
                }
                Err(e) => {
                    tracing::warn!(
                        line = lineno + 1,
                        error = %e,
                        "wake-ticket: skipping unparseable line"
                    );
                }
            }
        }
        Ok(order
            .into_iter()
            .filter_map(|id| by_id.remove(&id))
            .collect())
    }

    /// Atomically rewrite `wake-tickets.jsonl` to contain exactly `tickets`
    /// (tmp file + fsync + rename), `chmod 600`.
    fn write_compacted(dir: &Path, tickets: &[Ticket]) -> io::Result<()> {
        let tmp_path = dir.join(format!("{TICKETS_FILE}.tmp"));
        let final_path = dir.join(TICKETS_FILE);
        {
            let mut tmp = File::create(&tmp_path)?;
            for ticket in tickets {
                let line = serde_json::to_string(ticket)
                    .map_err(|e| io::Error::other(format!("ticket serialize error: {e}")))?;
                tmp.write_all(line.as_bytes())?;
                tmp.write_all(b"\n")?;
            }
            tmp.sync_all()?;
        }
        chmod_600(&tmp_path)?;
        fs::rename(&tmp_path, &final_path)?;
        Ok(())
    }

    /// Append one ticket line and fsync. Updates the in-memory index.
    fn append(&self, ticket: Ticket) -> io::Result<()> {
        let line = serde_json::to_string(&ticket)
            .map_err(|e| io::Error::other(format!("ticket serialize error: {e}")))?;
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.file.write_all(line.as_bytes())?;
        inner.file.write_all(b"\n")?;
        inner.file.sync_data()?;
        // First append to a freshly-created file — fix permissions now that
        // the inode exists (create_dir_all + append-open above may have
        // created it with the process umask).
        chmod_600(&self.dir.join(TICKETS_FILE))?;
        inner.index.insert(ticket.event_id.clone(), ticket);
        Ok(())
    }

    /// Tickets in `open` or `claimed` state — candidates for boot replay.
    /// Order is unspecified; callers sort/dedupe as needed.
    pub fn pending_for_replay(&self) -> Vec<Ticket> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .index
            .values()
            .filter(|t| matches!(t.state, TicketState::Open | TicketState::Claimed))
            .cloned()
            .collect()
    }

    /// Write `open` for a newly-accepted mention. No-op (logged) if the
    /// channel already has `MAX_PENDING_PER_CHANNEL` open+claimed tickets —
    /// mirrors the in-memory queue's own per-channel depth cap.
    #[allow(clippy::too_many_arguments)]
    pub fn write_open(
        &self,
        event: &nostr::Event,
        channel_id: Uuid,
        pubkey: &str,
        prompt_tag: &str,
        seen_at: u64,
    ) {
        let event_id = event.id.to_hex();
        {
            let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let pending_for_channel = inner
                .index
                .values()
                .filter(|t| {
                    t.channel_id == channel_id
                        && matches!(t.state, TicketState::Open | TicketState::Claimed)
                })
                .count();
            if pending_for_channel >= MAX_PENDING_PER_CHANNEL {
                tracing::warn!(
                    %channel_id,
                    cap = MAX_PENDING_PER_CHANNEL,
                    "wake-ticket: per-channel cap reached — not writing ticket \
                     (in-memory queue enforces the same cap independently)"
                );
                return;
            }
        }
        let ticket = Ticket {
            event_id: event_id.clone(),
            channel_id,
            created_at: event.created_at.as_secs(),
            pubkey: pubkey.to_string(),
            seen_at,
            state: TicketState::Open,
            event: event.clone(),
            prompt_tag: prompt_tag.to_string(),
        };
        if let Err(e) = self.append(ticket) {
            tracing::warn!(event_id = %event_id, error = %e, "wake-ticket: failed to write open");
        }
    }

    /// Transition tickets to `claimed` — a batch was handed to an agent.
    pub fn mark_claimed(&self, event_ids: &[String]) {
        self.transition(event_ids, TicketState::Claimed);
    }

    /// Event ids currently `claimed` for `channel_id`.
    ///
    /// The completion path (`lib.rs::handle_prompt_result`) doesn't carry
    /// the dispatched batch's event ids on the success path (`PromptResult`
    /// intentionally clears `batch` on `PromptOutcome::Ok` — nothing to
    /// requeue), so it looks up "what was claimed for this channel" here
    /// instead of threading ids through the pool/queue result plumbing.
    pub fn claimed_event_ids_for_channel(&self, channel_id: Uuid) -> Vec<String> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .index
            .values()
            .filter(|t| t.channel_id == channel_id && t.state == TicketState::Claimed)
            .map(|t| t.event_id.clone())
            .collect()
    }

    /// Transition tickets to `done` — `PromptOutcome::Ok` and nothing was
    /// requeued for these events. Compacted away on the next boot.
    pub fn mark_done(&self, event_ids: &[String]) {
        self.transition(event_ids, TicketState::Done);
    }

    /// Transition tickets to `drop` — dead-lettered, auth-failed, or the
    /// channel was removed. The line survives on disk for audit; it is
    /// never a replay candidate again.
    pub fn mark_drop(&self, event_ids: &[String]) {
        self.transition(event_ids, TicketState::Drop);
    }

    /// Drop a replay candidate that failed re-validation (membership, author
    /// gate, or rule match) without ever pushing it back into the queue.
    pub fn drop_unvalidated(&self, event_id: &str) {
        self.transition(
            std::slice::from_ref(&event_id.to_string()),
            TicketState::Drop,
        );
    }

    fn transition(&self, event_ids: &[String], state: TicketState) {
        for event_id in event_ids {
            let existing = {
                let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                inner.index.get(event_id).cloned()
            };
            let Some(mut ticket) = existing else {
                // No open/claimed/drop record — most commonly a non-mention
                // event in the same batch (never had a ticket) or a replay
                // survivor already compacted. Not an error.
                continue;
            };
            if ticket.state == state {
                continue; // idempotent — avoid a redundant fsync'd line
            }
            ticket.state = state;
            if let Err(e) = self.append(ticket) {
                tracing::warn!(
                    event_id = %event_id,
                    ?state,
                    error = %e,
                    "wake-ticket: failed to write state transition"
                );
            }
        }
    }
}

#[cfg(unix)]
fn chmod_dir_700(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn signed_event(keys: &nostr::Keys, content: &str) -> nostr::Event {
        nostr::EventBuilder::new(nostr::Kind::TextNote, content)
            .sign_with_keys(keys)
            .expect("sign")
    }

    #[test]
    fn open_write_and_replay_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = WakeTicketStore::open(tmp.path()).unwrap();
        let keys = nostr::Keys::generate();
        let event = signed_event(&keys, "hello");
        let channel_id = Uuid::new_v4();

        store.write_open(
            &event,
            channel_id,
            &keys.public_key().to_hex(),
            "@mention",
            now(),
        );

        let pending = store.pending_for_replay();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].event_id, event.id.to_hex());
        assert_eq!(pending[0].state, TicketState::Open);
        assert_eq!(pending[0].event.id, event.id);
    }

    #[test]
    fn claimed_then_done_removes_from_replay_candidates() {
        let tmp = tempfile::tempdir().unwrap();
        let store = WakeTicketStore::open(tmp.path()).unwrap();
        let keys = nostr::Keys::generate();
        let event = signed_event(&keys, "hi");
        let channel_id = Uuid::new_v4();
        let id = event.id.to_hex();

        store.write_open(
            &event,
            channel_id,
            &keys.public_key().to_hex(),
            "@mention",
            now(),
        );
        store.mark_claimed(std::slice::from_ref(&id));
        assert_eq!(store.pending_for_replay()[0].state, TicketState::Claimed);

        store.mark_done(std::slice::from_ref(&id));
        assert!(store.pending_for_replay().is_empty());
    }

    #[test]
    fn done_is_compacted_away_on_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let keys = nostr::Keys::generate();
        let event = signed_event(&keys, "hi");
        let channel_id = Uuid::new_v4();
        let id = event.id.to_hex();
        {
            let store = WakeTicketStore::open(tmp.path()).unwrap();
            store.write_open(
                &event,
                channel_id,
                &keys.public_key().to_hex(),
                "@mention",
                now(),
            );
            store.mark_done(std::slice::from_ref(&id));
            // Compaction is lazy (see `WakeTicketStore::open` docs) — `mark_done`
            // only appends the `done` transition line; the file still carries
            // both lines until the next `open()` rewrites it. In-memory state
            // is already correct here (covered by
            // `claimed_then_done_removes_from_replay_candidates`).
            assert!(store.pending_for_replay().is_empty());
        }

        let store = WakeTicketStore::open(tmp.path()).unwrap();
        let contents = fs::read_to_string(tmp.path().join(TICKETS_FILE)).unwrap();
        assert!(
            contents.is_empty(),
            "done ticket should be compacted away on reopen, got: {contents}"
        );
        assert!(store.pending_for_replay().is_empty());
    }

    #[test]
    fn drop_survives_compaction_but_is_not_a_replay_candidate() {
        let tmp = tempfile::tempdir().unwrap();
        let keys = nostr::Keys::generate();
        let event = signed_event(&keys, "hi");
        let channel_id = Uuid::new_v4();
        let id = event.id.to_hex();
        {
            let store = WakeTicketStore::open(tmp.path()).unwrap();
            store.write_open(
                &event,
                channel_id,
                &keys.public_key().to_hex(),
                "@mention",
                now(),
            );
            store.mark_drop(std::slice::from_ref(&id));
        }
        let contents = fs::read_to_string(tmp.path().join(TICKETS_FILE)).unwrap();
        assert!(
            !contents.trim().is_empty(),
            "drop ticket must survive compaction for audit"
        );

        let store = WakeTicketStore::open(tmp.path()).unwrap();
        assert!(store.pending_for_replay().is_empty());
    }

    #[test]
    fn open_and_claimed_survive_process_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let keys = nostr::Keys::generate();
        let channel_id = Uuid::new_v4();
        let open_event = signed_event(&keys, "one");
        let claimed_event = signed_event(&keys, "two");
        {
            let store = WakeTicketStore::open(tmp.path()).unwrap();
            store.write_open(
                &open_event,
                channel_id,
                &keys.public_key().to_hex(),
                "@mention",
                now(),
            );
            store.write_open(
                &claimed_event,
                channel_id,
                &keys.public_key().to_hex(),
                "@mention",
                now(),
            );
            store.mark_claimed(&[claimed_event.id.to_hex()]);
        }

        let store = WakeTicketStore::open(tmp.path()).unwrap();
        let mut pending = store.pending_for_replay();
        pending.sort_by_key(|t| t.event_id.clone());
        assert_eq!(pending.len(), 2);
        assert!(pending
            .iter()
            .any(|t| t.event_id == open_event.id.to_hex() && t.state == TicketState::Open));
        assert!(pending
            .iter()
            .any(|t| t.event_id == claimed_event.id.to_hex() && t.state == TicketState::Claimed));
    }

    #[test]
    fn second_open_on_same_dir_fails_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let _store = WakeTicketStore::open(tmp.path()).unwrap();
        let second = WakeTicketStore::open(tmp.path());
        assert!(second.is_err(), "dual-run must fail, not block or succeed");
    }

    #[test]
    fn lock_releases_on_drop_so_a_later_process_can_open() {
        let tmp = tempfile::tempdir().unwrap();
        {
            let _store = WakeTicketStore::open(tmp.path()).unwrap();
        }
        let reopened = WakeTicketStore::open(tmp.path());
        assert!(reopened.is_ok(), "lock must release when the store drops");
    }

    #[test]
    fn unparseable_line_is_skipped_not_fatal() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join(TICKETS_FILE),
            "not json\n{\"also\": \"not a ticket\"}\n",
        )
        .unwrap();
        let store = WakeTicketStore::open(tmp.path());
        assert!(store.is_ok());
        assert!(store.unwrap().pending_for_replay().is_empty());
    }

    #[test]
    fn last_write_wins_by_event_id_across_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let keys = nostr::Keys::generate();
        let event = signed_event(&keys, "hi");
        let channel_id = Uuid::new_v4();
        let id = event.id.to_hex();
        {
            let store = WakeTicketStore::open(tmp.path()).unwrap();
            store.write_open(
                &event,
                channel_id,
                &keys.public_key().to_hex(),
                "@mention",
                now(),
            );
            store.mark_claimed(std::slice::from_ref(&id));
            // Simulate a crash right after claim — no done/drop line yet.
        }
        let store = WakeTicketStore::open(tmp.path()).unwrap();
        let pending = store.pending_for_replay();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].state, TicketState::Claimed);
    }

    #[test]
    fn per_channel_cap_stops_writing_new_open_tickets() {
        let tmp = tempfile::tempdir().unwrap();
        let store = WakeTicketStore::open(tmp.path()).unwrap();
        let keys = nostr::Keys::generate();
        let channel_id = Uuid::new_v4();
        for i in 0..MAX_PENDING_PER_CHANNEL {
            let event = signed_event(&keys, &format!("msg {i}"));
            store.write_open(
                &event,
                channel_id,
                &keys.public_key().to_hex(),
                "@mention",
                now(),
            );
        }
        assert_eq!(store.pending_for_replay().len(), MAX_PENDING_PER_CHANNEL);

        let overflow = signed_event(&keys, "overflow");
        store.write_open(
            &overflow,
            channel_id,
            &keys.public_key().to_hex(),
            "@mention",
            now(),
        );
        assert_eq!(
            store.pending_for_replay().len(),
            MAX_PENDING_PER_CHANNEL,
            "cap must hold — overflow ticket must not be written"
        );
    }
}
