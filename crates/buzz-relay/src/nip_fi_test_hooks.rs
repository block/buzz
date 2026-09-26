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
//! | **CW8** (post-add_peer cancel → cleanup) | `audio/handler.rs` — `after_add_peer` fires immediately after `room.add_peer` succeeds and before the explicit `cancel.is_cancelled()` branch that calls `guard.release_before_commit()` | Delete `after_add_peer(...)` call | `arrived_rx` times out — test panics |
//! | **CW8** (post-add_peer cancel → cleanup) | same | Delete `room.remove_peer(peer_id)` from cleanup block | room entry persists — `room_after.is_none()` assertion panics |
//! | **CW8** (post-add_peer cancel → cleanup) | same | Delete `cleanup_if_empty(...)` from cleanup block | room entry persists — `room_after.is_none()` assertion panics |
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
// between registration and check. The invariant covers BOTH sides of the race:
//   (a) a concurrent disconnect sees the registered session and closes it (close
//       scan side) — exercised by the real `disconnect_nip_fi` call in the test,
//       which asserts exactly 1 session found at the hook window; OR
//   (b) the deny check fires here and finds the entry (check side) — exercised by
//       the is_cancelled + AuthorizationDenied NOTICE oracles.
//
// Mutation evidence (W_deny_straddle, W_audio_deny):
//   A) Delete `before_deny_set_check(...)` from auth.rs / audio/handler.rs →
//      handler never stalls → deny entry is inserted AFTER the check already
//      ran and missed it → connection is admitted → `is_cancelled()` assertion
//      panics.
//   B) Remove the `is_denied` check entirely → same outcome as (A).
//   C) Move `before_deny_set_check` to BEFORE `set_authenticated_pubkey` →
//      hook fires before registration → close-scan side fails (disconnect finds
//      0 sessions) → `assert_eq!(scan_count, 1)` panics regardless of whether
//      (b) still catches the deny.
make_hook!(deny_set_check_hook, before_deny_set_check);

// `before_first_audio_check_cancel`: fires in `handle_active_audio_connection`
// immediately before the first `check_cancel!()` invocation (after
// `enforce_relay_membership` returns and before Step 3 membership check).
// By this point the NIP-FI expiry task has already been spawned (line ~426),
// so tests can hold the handler here while the expiry task fires naturally,
// then release to let `check_cancel!()` drain the terminal channel and emit
// the policy close. Used by W_FIX1.
//
// Mutation evidence (W_FIX1):
//   A) Delete the `if let Some(reason) = nip_fi_close_reason` block from the
//      plain `check_cancel!()` arm → client receives only restricted JSON, no
//      close → W_FIX1 close assertion panics.
//   B) Move this hook to before `spawn_nip_fi_expiry_task` → expiry task fires
//      AFTER hook releases → cancel not set when check_cancel!() runs →
//      handler proceeds to membership check instead of returning →
//      W_FIX1 frame-0 assertion times out → panics.
make_hook!(
    audio_before_first_check_cancel_hook,
    before_first_audio_check_cancel
);

// `after_deny_set_check_passed`: fires in the audio handler immediately after the
// deny-set check block completes WITHOUT denying (i.e., the key passed). Used by
// `w_audio_deny_absent` to prove the absent key reached the post-check/membership
// gate without being denied or cancelled.
//
// Mutation evidence (W_audio_deny_absent):
//   A) Invert `is_denied` → the absent key is denied BEFORE this hook fires →
//      handler returns early → hook never fires → `arrived_rx` times out → panics.
//   B) Move the hook to before the deny check → fires unconditionally regardless
//      of denial; but the cancel assertion (not yet set) would still pass the
//      absent case until after release — use in combination with the active test.
make_hook!(
    audio_after_deny_check_passed_hook,
    after_deny_set_check_passed
);

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

// ── Observer-permit hook ───────────────────────────────────────────────────
// `before_observer_publication`: fires immediately before `acquire_effect` in
// `handle_agent_observer_event`, after all validation and owner checks.
// Used by the F1 witness: after expiry is armed and the handler reaches this
// hook, fire cancel so `acquire_effect` returns `SessionExpired` and the
// observer frame is NOT published.
//
// Mutation evidence (F1 witness):
//   A) Delete `#[cfg(test)] before_observer_publication(...)` call →
//      `arrived_rx` times out → test panics.
//   B) Remove `acquire_effect` from the observer handler →
//      handler calls publish_event even when expired → OK(true) sent →
//      assertion on the cancel-before-acquisition invariant panics.
//   C) Move hook to after `mark_local_event` (past the permit) →
//      hook fires after the irreversible side effect; expired gate
//      can no longer prevent delivery → publication counter shows 1 → panics.
make_hook!(observer_publication_hook, before_observer_publication);

// ── F4: add-peer gate-acquire hook ────────────────────────────────────────
// `before_add_peer_gate_acquire`: fires in `handle_active_audio_connection`
// immediately before `audio_gate.acquire_effect()` in Step 5 (add-peer).
// After auth + membership pass, the handler reaches this hook just before the
// gate acquire that may return `SessionExpired`.
//
// F4 add-peer test arms this hook, awaits arrival (handler at the gate),
// then expires the gate manually (`gate.expire()` or via cancellation).
// Releases the hook → handler resumes → `acquire_effect()` returns
// `SessionExpired` → handler calls `expiry_deny_terminal` + R2 drain +
// sends denial frame + close.  The WS client then observes:
//   frame 0: restricted JSON (Audio authorization-denied payload)
//   frame 1: 1008 POLICY close
//
// Removing `expiry_deny_terminal` from the `SessionExpired` arm makes frame 0
// either absent or a close-only, which breaks the frame-0 equality assertion.
// Removing R2 drain makes frame 0 absent (denial queued but never drained).
//
// Mutation evidence:
//   A) Remove `expiry_deny_terminal` from the add-peer SessionExpired arm →
//      frame 0 is not the denial JSON → wire assertion panics.
//   B) Remove R2 drain (`while let Ok(msg) = terminal_ctrl_rx.try_recv()`) →
//      denial queued but never sent → frame 0 is missing → timeout panics.
//   C) Delete `before_add_peer_gate_acquire(...)` from handler →
//      arrived_rx times out → test panics (proves hook is at the right seam).
make_hook!(
    audio_add_peer_gate_acquire_hook,
    before_add_peer_gate_acquire
);

// ── R2: room-ended error arm — admin fires before lifecycle_cancel ─────────
// `before_room_ended_lifecycle_cancel`: fires in the `AdmissionError::Ended`
// arm of `handle_active_audio_connection`, AFTER the room_ended error frame
// is sent to the client but BEFORE `lifecycle_cancel()` is called.
//
// R2 test arms this hook, awaits arrival, then calls
// `registry.disconnect_nip_fi` from the test (admin path: wins reason,
// enqueues denial frame, fires cancel).  Releases the hook → handler calls
// `lifecycle_cancel()` (loses reason, but cancel is set), awaits expiry,
// then the R2 drain delivers the pre-queued denial frame + close.  The WS
// client observes the denial frame + POLICY close after the room_ended error.
//
// Mutation evidence:
//   A) Remove R2 drain from the Ended arm → denial queued but never sent →
//      client never sees the denial frame → timeout after room_ended → panics.
//   B) Delete `before_room_ended_lifecycle_cancel(...)` from handler →
//      `arrived_rx` times out → test panics (proves hook is at the right seam).
//   C) Move R2 drain to BEFORE guard.release_before_commit() → out-of-order
//      delivery — but functionally equivalent; test checks frame ORDER, not
//      guard timing.
make_hook!(
    audio_room_ended_lifecycle_cancel_hook,
    before_room_ended_lifecycle_cancel
);

// ── R2: drain_terminal!() — producer-paused race witness ──────────────────
// `before_not_a_member_drain_terminal`: fires in `handle_active_audio_connection`
// in the `check_membership_for_admission` error arm (not-a-member), AFTER the
// diagnostic "not a member" frame is sent but BEFORE `drain_terminal!()` is called.
//
// This is the hook that makes the R2 race witness specifically exercise the
// `drain_terminal!()` macro (which calls `lifecycle_cancel()` internally).
// It differs from `before_room_ended_lifecycle_cancel`, which is on a path that
// calls explicit `lifecycle_cancel()` before `finalize_drain!()`.
//
// R2 race test sequence:
//   1. Arm this hook + arm `cancel_race_test_hook` on the control.
//   2. Connect a non-member client: handler reaches this hook, pauses.
//   3. Test fires `disconnect_nip_fi` in a thread.  Admin wins reason, fires
//      `cancel_race_test_hook` (lock held, before try_send) → admin pauses.
//   4. Test awaits admin's ready signal.
//   5. Test releases this hook → handler proceeds to `drain_terminal!()` →
//      `lifecycle_cancel()` tries to acquire the transition lock → BLOCKS
//      (admin holds it).
//   6. Test releases admin's hook → admin completes `try_send`, drops lock.
//   7. `lifecycle_cancel()` unblocks → drains → denial frame delivered → close.
//
// Falsification: remove `lifecycle_cancel()` from the `drain_terminal!()` macro →
//   handler drains immediately (lock not acquired, admin may not have enqueued yet)
//   → `try_recv()` returns Err(Empty) → denial frame absent → only close frame
//   seen by client → frame-1 assertion panics.
//
// Mutation evidence:
//   A) Delete `before_not_a_member_drain_terminal(...)` from handler →
//      `arrived_rx` times out → test panics (proves hook is at the right seam).
//   B) Remove `lifecycle_cancel()` from `drain_terminal!()` →
//      handler does not block on admin's lock → drain races admin's try_send
//      → denial frame absent after close → frame-1 timeout → panics.
make_hook!(
    audio_not_a_member_drain_terminal_hook,
    before_not_a_member_drain_terminal
);
// ── F5: REQ permit-acquired hook ──────────────────────────────────────────
// `after_req_permit_acquired`: fires in `handlers/req.rs` immediately AFTER
// `acquire_effect()` returns `Ok(permit)` (permit is now held) but BEFORE
// `sub_registry.register_scoped` / `register_channels_scoped`.
//
// This hook establishes a permit-holding REQ handler that has not yet
// registered its subscription. The F5 loopback teardown witness uses this
// to drive a real `handle_active_connection` to this state, fire admin
// disconnect, then release — letting the real connection epilogue
// (`remove_connection` + `release_topic`) run and asserting zero topic
// refcounts remain.
//
// Mutation evidence (F5 loopback test):
//   A) Remove `gate.quiesce().await` from the expiry task cancel arm →
//      expiry task exits before REQ registers its subscription →
//      connection epilogue runs `remove_connection` on an empty set →
//      the subscription registered after task exit orphans permanently →
//      but the test actually fires disconnect WHILE the permit is held, so
//      quiescence is still the binding contract.
//   B) Remove `acquire_effect()` from req.rs → handler inserts subscription
//      without the permit → quiescence barrier not crossed → subscription
//      is registered after task exits → `remove_connection` finds 0 entries →
//      topic_refcount stays > 0 → assertion panics.
//   C) Delete `after_req_permit_acquired(...)` from req.rs →
//      `arrived_rx` times out → test panics (proves hook is at correct seam).
make_hook!(req_permit_acquired_hook, after_req_permit_acquired);
