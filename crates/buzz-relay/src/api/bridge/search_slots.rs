//! Per-user cap on bridge searches in flight.
//!
//! Typeahead starts a search on almost every keystroke. The client aborts the
//! previous request, but an abort that never reaches the relay (an
//! intermediary that keeps the upstream request open) leaves every one of
//! them running. Each `(community, pubkey)` may have [`PER_USER`] searches in
//! flight; the next one supersedes the oldest, whose request then fails with
//! [`Superseded`] (409, never 429: clients read 429 as relay-wide
//! back-pressure and hold all their relay traffic). Dropping the superseded search cancels its statement on
//! the server (see `buzz_search::cancel`).
//!
//! Newest wins rather than rejecting the newcomer: the newest keystroke is
//! the one the user is waiting for. The cap is above one so that sessions
//! sharing a pubkey (several agent sessions, parallel people and message
//! search) keep their parallel searches.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use axum::http::StatusCode;
use axum::Json;
use buzz_core::CommunityId;
use tokio_util::sync::CancellationToken;

use crate::api::api_error;

/// Bridge searches one `(community, pubkey)` may have in flight.
pub(crate) const PER_USER: usize = 4;

type Key = (CommunityId, [u8; 32]);

/// A search gave way to a newer one from the same user.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Superseded;

/// Body of a superseded search's response.
const SUPERSEDED: &str = "search superseded by a newer search";

/// 409, not 429: desktop and the CLI treat any 429 (and any `rate-limited:`
/// body) as relay-wide back-pressure, so a 429 here would stall every relay
/// request the client makes, or retry the search it just replaced.
impl From<Superseded> for (StatusCode, Json<serde_json::Value>) {
    fn from(_: Superseded) -> Self {
        api_error(StatusCode::CONFLICT, SUPERSEDED)
    }
}

/// In-flight bridge searches per user, oldest first.
#[derive(Default)]
pub(crate) struct SearchSlots {
    in_flight: Mutex<HashMap<Key, VecDeque<(u64, CancellationToken)>>>,
    next_id: AtomicU64,
}

impl SearchSlots {
    /// Run `search` in one of `key`'s slots. Returns `Err(Superseded)`, and
    /// drops `search`, if [`PER_USER`] newer searches from `key` start first.
    pub(crate) async fn run<F: Future>(
        &self,
        community: CommunityId,
        pubkey: [u8; 32],
        search: F,
    ) -> Result<F::Output, Superseded> {
        let slot = self.claim((community, pubkey));
        // Polled first: a search that finishes as it is superseded still
        // returns its result.
        tokio::select! {
            biased;
            output = search => Ok(output),
            () = slot.superseded.cancelled() => Err(Superseded),
        }
    }

    fn claim(&self, key: Key) -> Slot<'_> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let superseded = CancellationToken::new();
        let mut in_flight = self.lock();
        let queue = in_flight.entry(key).or_default();
        queue.push_back((id, superseded.clone()));
        if queue.len() > PER_USER {
            if let Some((_, oldest)) = queue.pop_front() {
                oldest.cancel();
            }
        }
        Slot {
            slots: self,
            key,
            id,
            superseded,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Key, VecDeque<(u64, CancellationToken)>>> {
        // A panic while holding this lock leaves the map consistent: every
        // critical section is a single push, pop or retain.
        self.in_flight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(test)]
    fn users(&self) -> usize {
        self.lock().len()
    }
}

/// One search's place in its user's queue; leaving it (finished, failed,
/// dropped or superseded) frees the place.
struct Slot<'a> {
    slots: &'a SearchSlots,
    key: Key,
    id: u64,
    superseded: CancellationToken,
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        let mut in_flight = self.slots.lock();
        if let Some(queue) = in_flight.get_mut(&self.key) {
            // By id: a superseded slot is already gone, and must not remove
            // the newer search that took its place.
            queue.retain(|(id, _)| *id != self.id);
            if queue.is_empty() {
                in_flight.remove(&self.key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::pin;
    use std::task::{Context, Poll};

    fn user(n: u8) -> (CommunityId, [u8; 32]) {
        (CommunityId::from_uuid(uuid::Uuid::from_u128(1)), [n; 32])
    }

    /// Poll `fut` once.
    fn poll_once<F: Future>(fut: std::pin::Pin<&mut F>) -> Poll<F::Output> {
        fut.poll(&mut Context::from_waker(std::task::Waker::noop()))
    }

    #[tokio::test]
    async fn the_search_past_the_cap_supersedes_the_oldest() {
        let slots = SearchSlots::default();
        let (community, me) = user(1);
        let (_, other) = user(2);
        let mut held = Vec::new();
        for _ in 0..PER_USER {
            held.push(Box::pin(slots.run(
                community,
                me,
                std::future::pending::<()>(),
            )));
        }
        let mut theirs = pin!(slots.run(community, other, std::future::pending::<()>()));
        for search in &mut held {
            assert!(poll_once(search.as_mut()).is_pending());
        }
        assert!(poll_once(theirs.as_mut()).is_pending());

        let mut newest = pin!(slots.run(community, me, std::future::pending::<()>()));
        assert!(poll_once(newest.as_mut()).is_pending());
        assert_eq!(poll_once(held[0].as_mut()), Poll::Ready(Err(Superseded)));
        for search in &mut held[1..] {
            assert!(
                poll_once(search.as_mut()).is_pending(),
                "only the oldest gives way"
            );
        }
        assert!(
            poll_once(theirs.as_mut()).is_pending(),
            "other users keep their searches"
        );
        assert!(poll_once(newest.as_mut()).is_pending());
    }

    #[tokio::test]
    async fn a_finished_search_frees_its_slot() {
        let slots = SearchSlots::default();
        let (community, me) = user(1);
        assert_eq!(slots.run(community, me, async { 7 }).await, Ok(7));
        let mut held = Vec::new();
        for _ in 0..PER_USER {
            let mut search = Box::pin(slots.run(community, me, std::future::pending::<()>()));
            assert!(poll_once(search.as_mut()).is_pending());
            held.push(search);
        }
        // A dropped search frees its slot too, so the next one displaces no one.
        drop(held.pop());
        let mut next = pin!(slots.run(community, me, std::future::pending::<()>()));
        assert!(poll_once(next.as_mut()).is_pending());
        for search in &mut held {
            assert!(poll_once(search.as_mut()).is_pending());
        }
    }

    #[tokio::test]
    async fn a_superseded_search_leaves_its_successor_in_place() {
        let slots = SearchSlots::default();
        let (community, me) = user(1);
        let mut oldest = Box::pin(slots.run(community, me, std::future::pending::<()>()));
        assert!(poll_once(oldest.as_mut()).is_pending());
        let mut rest = Vec::new();
        for _ in 0..PER_USER {
            let mut search = Box::pin(slots.run(community, me, std::future::pending::<()>()));
            assert!(poll_once(search.as_mut()).is_pending());
            rest.push(search);
        }
        assert_eq!(poll_once(oldest.as_mut()), Poll::Ready(Err(Superseded)));
        drop(oldest);
        // Still exactly PER_USER in flight: one more displaces the oldest survivor.
        let mut next = pin!(slots.run(community, me, std::future::pending::<()>()));
        assert!(poll_once(next.as_mut()).is_pending());
        assert_eq!(poll_once(rest[0].as_mut()), Poll::Ready(Err(Superseded)));
        for search in &mut rest[1..] {
            assert!(poll_once(search.as_mut()).is_pending());
        }
    }

    #[test]
    fn a_superseded_search_is_a_conflict_not_back_pressure() {
        let (status, Json(body)) = Superseded.into();
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            body,
            serde_json::json!({ "error": "search superseded by a newer search" })
        );
    }

    #[tokio::test]
    async fn no_entry_outlives_its_searches() {
        let slots = SearchSlots::default();
        for n in 0..3 {
            let (community, pubkey) = user(n);
            let mut search = Box::pin(slots.run(community, pubkey, std::future::pending::<()>()));
            assert!(poll_once(search.as_mut()).is_pending());
            assert_eq!(slots.users(), 1);
            drop(search);
            assert_eq!(slots.users(), 0);
        }
    }
}
