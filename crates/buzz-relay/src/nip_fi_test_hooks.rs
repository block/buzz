//! Test-only barriers for NIP-FI B2 witness tests.
//!
//! Each function is a named production hook that is inert in production
//! (`#[cfg(test)]` guards ensure zero-cost at runtime) but acts as a
//! deterministic barrier in tests. A test arms the gate, dispatches work,
//! waits for the arrived notification, fires expiry, then releases the gate.
//!
//! Pattern (same as `publish_test_hooks` in `side_effects.rs`):
//! - `arm(community)` → `(arrived_rx, release_notify)`
//! - Production code calls `before_X(community).await`
//! - Test awaits `arrived_rx.await` → knows production reached the hook
//! - Test fires expiry
//! - Test calls `release_notify.notify_one()` → production proceeds
//!
//! Only one gate per community-slot is supported at a time (static Mutex<HashMap>).
//! Tests using different communities can run concurrently — each gets its own gate.
//! Tests using the same community must not run concurrently (they will interfere).
//!
//! # Per-witness mutation-red table
//!
//! Every witness listed below follows the same structure:
//!
//! | Witness | Hook location (production file:line) | One-line mutation | Failing assertion |
//! |---------|--------------------------------------|-------------------|-------------------|
//! | **W1** (auth barrier) | `handlers/auth.rs:319` — immediately before `acquire_effect()` in AUTH commit path | Delete `before_auth_commit(...)` call | `arrived_rx` times out → test panics |
//! | **W1** (auth barrier) | same | Remove `acquire_effect()` from auth.rs | `auth_state is NOT Authenticated` → assertion panics |
//! | **W1** (auth barrier) | same | Change gate to `off_mode` | same as above |
//! | **W2** (event barrier) | `handlers/event.rs:784` — immediately before `acquire_effect()` in event ingest path | Delete `before_event_ingest(...)` call | `arrived_rx` times out → test panics |
//! | **W2** (event barrier) | same | Remove `acquire_effect()` from event.rs | "session expired" OK(false) not sent → first `try_recv` panics |
//! | **W2** (event barrier) | same | Change gate to `off_mode` | same as above |
//! | **W3** (REQ barrier) | `handlers/req.rs:280` — immediately before `acquire_effect()` in REQ path | Delete `before_req_registration(...)` call | `arrived_rx` times out → test panics |
//! | **W3** (REQ barrier) | same | Remove `acquire_effect()` from req.rs | subscription IS inserted → `subs.is_empty()` panics |
//! | **W3** (REQ barrier) | same | Change gate to `off_mode` | same as above |
//! | **W4** (COUNT barrier) | `handlers/count.rs:112` — immediately before `acquire_effect()` in COUNT path | Delete `before_count_query(...)` call | `arrived_rx` times out → test panics |
//! | **W4** (COUNT barrier) | same | Remove `acquire_effect()` from count.rs | CLOSED message changes from "session expired" → assertion panics |
//! | **W4** (COUNT barrier) | same | Change gate to `off_mode` | no CLOSED sent → `try_recv` returns `Err` → assertion panics |
//! | **W5** (audio B1 expired-at-pairing) | `audio/handler.rs`, B1 deadline check after NIP-42 auth | Remove the already-expired deadline check | frame text changes to "not a relay member" → byte assertion panics |
//! | **W6** (audio B1 mid-admission) | `audio/handler.rs`, biased `cancel.cancelled()` in auth select | Remove `_ = cancel.cancelled() => return` | handler proceeds to auth exchange; close assertion fires on 3s timeout |
//! | **W7** (audio B3 expiry writer) | `nip_fi_session::spawn_nip_fi_expiry_task`, audio enqueue | Delete the audio denial enqueue | `frames[0]` is not the expected restricted JSON → assertion panics |
//! | **W8** (audio membership barrier) | `audio/handler.rs:1572` — entry of `check_membership_for_admission` | Delete `before_membership_check(...)` call | `arrived_rx` times out → test panics |
//! | **W8** (audio membership barrier) | same | Move hook to after `state.db.get_channel()` | DB error fires before hook on lazy pool → `arrived_rx` times out |
//! | **W9** (audio participant-commit barrier) | `audio/handler.rs:1796` — between uncommitted 48101 insert and `acquire_effect()` | Delete `before_participant_commit(...)` call | `arrived_rx` times out — test panics |
//! | **W9** (audio participant-commit barrier) | same | Remove `tx.rollback()` from `SessionExpired` branch | sqlx rolls back on drop regardless — mutation does NOT change test outcome (explicit rollback is belt-and-suspenders); covered by W9C instead |
//! | **W9** (audio participant-commit barrier) | same | Remove `acquire_effect()` entirely | commit proceeds despite cancel — row committed — row-count assertion panics |
//! | **W10** (concurrent committers, different pubkeys) | same as W9 | Delete `before_participant_commit(...)` call | `arrived_rx` times out — test panics |
//! | **W10** (concurrent committers, different pubkeys) | same | Remove `acquire_effect()` from `commit_participant_join` | second task commits too — two rows present — row-count assertion panics |
//! | **W10-reaffirm** (same pubkey twice) | same as W9 | Delete `before_participant_commit(...)` call | `arrived_rx` times out — test panics |
//! | **CW5** (AutoAddRequired joint-tx rollback) | `audio/handler.rs` — `before_participant_commit` fires after BOTH membership insert AND 48101 insert are in the uncommitted tx | Delete `before_participant_commit(...)` call | `arrived_rx` times out — test panics |
//! | **CW5** (AutoAddRequired joint-tx rollback) | same | Remove `acquire_effect()` from `commit_participant_join` | both rows committed — membership row-count assertion panics |
//! | **CW5** (AutoAddRequired joint-tx rollback) | same | Change `membership_admission` to `Existing` | auto-add path never entered; membership seam not covered — test fails at isolation |
//! | **CW5-variant** (concurrent external membership add) | `audio/handler.rs` — `before_membership_lock` fires inside the `AutoAddRequired` branch immediately before the channel membership lock | Delete `before_membership_lock(...)` call | `arrived_rx` times out — test panics |
//! | **CW5-variant** (concurrent external membership add) | same | Remove the `still_absent` re-read and always insert | external membership may be double-written (ON CONFLICT behaviour) — re-read path is the contract seam; removing it bypasses the contract |
//! | **CW5-variant** (concurrent external membership add) | same | Remove the `if still_absent { insert }` guard | same as above — auto-add fires unconditionally alongside the external row |
//! | **CW8** (post-add_peer cancel → cleanup) | `audio/handler.rs` — `after_add_peer` fires immediately after `room.add_peer` succeeds and before `check_cancel!(cleanup:...)` | Delete `after_add_peer(...)` call | `arrived_rx` times out — test panics |
//! | **CW8** (post-add_peer cancel → cleanup) | same | Delete `room.remove_peer(peer_id)` from cleanup block | room is non-empty — `room.is_empty()` assertion panics |
//! | **CW8** (post-add_peer cancel → cleanup) | same | Move `after_add_peer` hook to before `room.add_peer` | cancel fires before add_peer — check_cancel! exits without cleanup arm — room empty but hook fired at wrong seam |
//! | **CW10** (commit-won/quiescence: expiry blocked at barrier) | `audio/handler.rs` — `after_participant_fanout` fires after `tx.commit()` + fan-out, before `_permit` drops | Delete `after_participant_fanout(...)` call | `arrived_rx` times out — test panics |
//! | **CW10** (commit-won/quiescence: expiry blocked at barrier) | same | Remove `acquire_effect()` from `commit_participant_join` | permit never held — expiry completes before hook fires — `expire_done` is true before check — "expiry must be blocked" assertion panics |
//! | **CW10-full** (full-handler lifecycle: committed join → exactly one 48102) | `audio/handler.rs` — full `handle_active_audio_connection` via WS; hook at `after_participant_fanout`, then disconnect triggers normal teardown | Remove `emit_participant_event(48102, ...)` from handler epilogue | 48102 count stays 0 — assertion panics |
//! | **CW10-full** (full-handler lifecycle) | same | Remove `room.remove_peer_and_check_ended` from teardown | room entry persists — `audio_rooms.get()` returns Some — room assertion panics |
//! | **CW6** (guard-level: unattached lease released on pre-commit exit) | `audio/handler.rs` — `HuddleAdmissionGuard::release_before_commit` with injected `CountingDir` double (no Redis/mesh required) | Remove `if let Some((lease, directory)) = self.lease.take()` block from `release_before_commit` | `directory.release()` never called — `release_calls` stays 0 — assertion panics |
//! | **CW6** (guard-level: unattached lease released on pre-commit exit) | same | Short-circuit `release_before_commit` to return immediately before the lease block | same as above — `release_calls` stays 0 — assertion panics |
//! | **CW7** (guard-level: clean close sent on remote stream pre-commit exit) | `audio/handler.rs` — `HuddleAdmissionGuard::release_before_commit` with injected `RecordingSend` stub MeshStream + `RemoteHuddleSession::for_test` | Remove `if let (Some(session), Some(ref mut stream)) = ...` block from `release_before_commit` | `send_frame` never called — `goodbye_sent` is false — assertion panics |
//! | **CW7** (guard-level: clean close sent on remote stream pre-commit exit) | same | Swap `UnregisterPeer` and `Goodbye` frame order in `send_clean_close` | frames recorded in wrong order — assertion on Goodbye position panics |
//!
//! # Teardown ordering (quiescence citations)
//!
//! The quiescence requirement from the contract (e5bc0382): the expiry task must complete
//! (i.e., acquire and release the write guard after cancellation) before subscription/peer
//! cleanup runs. This prevents post-`remove_connection` subscription leaks.
//!
//! **Root WS** (`connection.rs:449-453`):
//! ```text
//! if let Some(task) = nip_fi_expiry_task { let _ = task.await; }  // line 449
//! for removed in state.sub_registry.remove_connection(...)  // line 453 — after expiry
//! ```
//!
//! **Audio WS** (`audio/handler.rs:1128-1138`):
//! ```text
//! if let Some(expiry_task) = nip_fi_audio_expiry_task { let _ = expiry_task.await; }  // line 1128
//! room.remove_peer_and_check_ended(peer_id)  // line 1138 — after expiry
//! ```
//!
//! **Pre-existing cleanup helpers** (audio expiry path):
//! - `send_clean_close` (`audio/join.rs`) — sends WS close frame for remote session path
//! - `cleanup_if_empty` (`audio/rooms.rs`) — removes room when peer count drops to zero
//! - `room.remove_peer` (`audio/room.rs`) — removes peer from in-memory room roster

use buzz_core::CommunityId;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use tokio::sync::{oneshot, Notify};

struct Gate {
    arrived: oneshot::Sender<()>,
    release: Arc<Notify>,
}

macro_rules! make_hook {
    ($mod_name:ident, $fn_name:ident) => {
        pub(crate) mod $mod_name {
            use super::*;

            // Keyed by CommunityId so concurrent tests with different communities
            // can arm independent gates without overwriting each other.
            static GATE: LazyLock<Mutex<HashMap<CommunityId, Gate>>> =
                LazyLock::new(|| Mutex::new(HashMap::new()));

            /// Arm a one-shot barrier for `community`.
            ///
            /// Returns `(arrived_rx, release)`. Await `arrived_rx` to know when
            /// the production code has reached this hook; call `release.notify_one()`
            /// to let it continue.
            pub(crate) fn arm(community: CommunityId) -> (oneshot::Receiver<()>, Arc<Notify>) {
                let (tx, rx) = oneshot::channel();
                let release = Arc::new(Notify::new());
                GATE.lock().unwrap().insert(
                    community,
                    Gate {
                        arrived: tx,
                        release: release.clone(),
                    },
                );
                (rx, release)
            }

            pub(crate) async fn trigger(community: CommunityId) {
                let gate = GATE.lock().unwrap().remove(&community);
                if let Some(g) = gate {
                    let _ = g.arrived.send(());
                    g.release.notified().await;
                }
            }
        }

        pub(crate) async fn $fn_name(community: CommunityId) {
            $mod_name::trigger(community).await;
        }
    };
}

make_hook!(auth_commit_hook, before_auth_commit);
make_hook!(event_ingest_hook, before_event_ingest);
make_hook!(req_registration_hook, before_req_registration);
make_hook!(count_query_hook, before_count_query);

// ── Audio B1 hooks ─────────────────────────────────────────────────────────
// `before_membership_check`: fires between NIP-42 pairing and the membership
// DB read inside `check_membership_for_admission`. Arms expiry here → proves
// that a cancellation before membership check produces zero DB side effects.
//
// `before_membership_lock`: fires inside the AutoAddRequired branch of
// `commit_participant_join`, immediately before
// `acquire_channel_membership_lock_in_transaction`. Arms an external
// membership insert here → proves that a concurrent add is observed by the
// re-read and the auto-add insert is skipped, leaving membership preserved.
//
// `before_participant_commit`: fires between the 48101 insert and the
// `acquire_effect()` + `tx.commit()` inside `commit_participant_join`. Arms
// expiry here → proves that a cancellation before the permit acquisition
// rolls back the transaction and produces zero post-expiry 48101/membership
// writes.
//
// `after_participant_fanout`: fires inside `commit_participant_join` after the
// 48101 is committed AND fan-out is complete but BEFORE `_permit` drops.
// Used by CW10: arms expiry here → proves expiry is blocked at the write
// guard while the permit is held; releasing the hook drops the permit and
// unblocks expiry.
//
// `after_add_peer`: fires in `handle_active_audio_connection` immediately
// after a successful `room.add_peer` call and before the subsequent
// `check_cancel!` fence. Arms cancel here → proves the cleanup branch
// (`room.remove_peer` + `cleanup_if_empty`) runs before the handler returns.
make_hook!(audio_membership_check_hook, before_membership_check);
make_hook!(audio_membership_lock_hook, before_membership_lock);
make_hook!(audio_participant_commit_hook, before_participant_commit);
make_hook!(audio_participant_fanout_hook, after_participant_fanout);
make_hook!(audio_add_peer_hook, after_add_peer);

// ── Deny-set admission hooks ───────────────────────────────────────────────
// `before_deny_set_check`: fires in BOTH the root WS handler (handlers/auth.rs)
// and the audio handler (audio/handler.rs), immediately AFTER
// `set_authenticated_pubkey`/`audio_post_auth_register` (registration) and
// immediately BEFORE the `is_denied(iss, k, now)` call.
//
// The straddle witness arms this gate, then inserts a deny entry in the window
// between registration and check. The invariant: either
//   (a) a concurrent disconnect sees the registered session and closes it (close
//       scan side), OR
//   (b) the deny check fires here and finds the entry (check side).
// This test exercises path (b): the entry is inserted AFTER registration but
// BEFORE the check — the check sees it and closes the connection.
//
// Mutation evidence (W_deny_straddle, W_audio_deny):
//   A) Delete `before_deny_set_check(...)` from auth.rs / audio/handler.rs →
//      handler never stalls → deny entry is inserted AFTER the check already
//      ran and missed it → connection is admitted → `is_cancelled()` assertion
//      panics.
//   B) Remove the `is_denied` check entirely → same outcome as (A).
//   C) Move `before_deny_set_check` to BEFORE `set_authenticated_pubkey` →
//      hook fires before registration → straddle semantics violated (close scan
//      cannot see the session) → test still passes because (b) side still works,
//      but the barrier witness is no longer at the correct seam.
make_hook!(deny_set_check_hook, before_deny_set_check);

// ── Publication-attempt counter ────────────────────────────────────────────
// `before_event_publish`: fires immediately before `state.pubsub.publish_event`
// in `dispatch_persistent_event_inner`. Used by W2: after handle_event returns
// under session-expired, assert this counter is 0 — proves `publish_event` was
// never called (real publication boundary, not a proxy).
//
// Mutation evidence (W2):
//   Remove `acquire_effect()` from event.rs → ingest_event is called →
//   dispatch_persistent_event_inner runs → before_event_publish fires →
//   counter = 1 → `assert_eq!(publish_count, 0)` panics.
pub(crate) mod event_publish_counter {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTERS: LazyLock<Mutex<HashMap<CommunityId, Arc<AtomicU32>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    /// Register a counter for `community` and return it. The counter starts at 0
    /// and is incremented each time `before_event_publish` fires for this community.
    pub(crate) fn register(community: CommunityId) -> Arc<AtomicU32> {
        let counter = Arc::new(AtomicU32::new(0));
        COUNTERS.lock().unwrap().insert(community, counter.clone());
        counter
    }

    /// Deregister the counter for `community` (call after the test assertion).
    pub(crate) fn deregister(community: CommunityId) {
        COUNTERS.lock().unwrap().remove(&community);
    }

    pub(crate) fn increment(community: CommunityId) {
        if let Some(counter) = COUNTERS.lock().unwrap().get(&community) {
            counter.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub(crate) fn before_event_publish(community: CommunityId) {
    event_publish_counter::increment(community);
}
