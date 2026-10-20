//! Request registry for thread catch-up generation: request-ID validation, a
//! small cap on concurrent generations, and cancellation that also wins when
//! it arrives before the generation registers.
//!
//! A cancel for an ID that is not running leaves a short-lived tombstone. A
//! later start with that ID consumes the tombstone and is refused, so the
//! person's cancellation holds even if the cancel IPC was scheduled first.
//! Tombstones are bounded in number and age and never occupy active slots.
//! The registry holds only request IDs, never thread content.
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

pub(super) const MAX_REQUEST_ID_CHARS: usize = 128;
pub(super) const MAX_ACTIVE_REQUESTS: usize = 4;
const MAX_TOMBSTONES: usize = 64;
const TOMBSTONE_TTL: Duration = Duration::from_secs(90);

/// Error for a start whose cancel arrived first; `cancelled:` is the stable
/// prefix the frontend already handles.
pub(super) const CANCELLED_BEFORE_START: &str =
    "cancelled: the recap was cancelled before it started";

#[derive(Default)]
struct Registry {
    active: HashMap<String, CancellationToken>,
    /// Cancels for IDs that were not running, with when they arrived.
    cancelled: HashMap<String, Instant>,
}

impl Registry {
    fn prune(&mut self, now: Instant) {
        self.cancelled
            .retain(|_, at| now.saturating_duration_since(*at) < TOMBSTONE_TTL);
    }

    fn start(&mut self, id: &str, now: Instant) -> Result<CancellationToken, String> {
        validate_request_id(id)?;
        self.prune(now);
        if self.cancelled.remove(id).is_some() {
            return Err(CANCELLED_BEFORE_START.into());
        }
        if self.active.contains_key(id) {
            return Err("a recap with this request id is already running".into());
        }
        if self.active.len() >= MAX_ACTIVE_REQUESTS {
            return Err("Too many recaps are running; wait for one to finish.".into());
        }
        let token = CancellationToken::new();
        self.active.insert(id.to_string(), token.clone());
        Ok(token)
    }

    fn cancel(&mut self, id: &str, now: Instant) -> Result<(), String> {
        validate_request_id(id)?;
        if let Some(token) = self.active.get(id) {
            token.cancel();
            return Ok(());
        }
        self.prune(now);
        // At capacity, drop the oldest tombstone: its start is the least
        // likely to still be in flight.
        if self.cancelled.len() >= MAX_TOMBSTONES && !self.cancelled.contains_key(id) {
            if let Some(oldest) = self
                .cancelled
                .iter()
                .min_by_key(|(_, at)| **at)
                .map(|(id, _)| id.clone())
            {
                self.cancelled.remove(&oldest);
            }
        }
        self.cancelled.insert(id.to_string(), now);
        Ok(())
    }
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(Mutex::default)
}

fn lock(registry: &Mutex<Registry>) -> std::sync::MutexGuard<'_, Registry> {
    registry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(super) fn validate_request_id(id: &str) -> Result<(), String> {
    let ok = !id.is_empty()
        && id.len() <= MAX_REQUEST_ID_CHARS
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if ok {
        Ok(())
    } else {
        Err("invalid recap request id".into())
    }
}

/// One registered generation; unregisters itself on every exit path.
pub(super) struct ActiveRequest {
    registry: &'static Mutex<Registry>,
    id: String,
    pub(super) token: CancellationToken,
}

impl ActiveRequest {
    /// Register a generation, refusing invalid, duplicate, over-limit, or
    /// already-cancelled request IDs.
    pub(super) fn start(id: &str) -> Result<Self, String> {
        Self::start_in(registry(), id, Instant::now())
    }

    fn start_in(
        registry: &'static Mutex<Registry>,
        id: &str,
        now: Instant,
    ) -> Result<Self, String> {
        let token = lock(registry).start(id, now)?;
        Ok(Self {
            registry,
            id: id.to_string(),
            token,
        })
    }
}

impl Drop for ActiveRequest {
    fn drop(&mut self) {
        lock(self.registry).active.remove(&self.id);
    }
}

/// Cancel a running generation now, or make a not-yet-started one refuse to
/// start. Cancelling a finished or unknown ID only leaves a tombstone that
/// expires unused.
pub(super) fn cancel_request(id: &str) -> Result<(), String> {
    lock(registry()).cancel(id, Instant::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An isolated registry, so cap and expiry cases cannot race other tests
    /// that share the process-wide one.
    fn local() -> &'static Mutex<Registry> {
        Box::leak(Box::default())
    }

    fn fresh() -> String {
        format!("req-{}", uuid::Uuid::new_v4())
    }

    #[test]
    fn request_ids_are_validated_on_start_and_cancel() {
        let too_long = "x".repeat(MAX_REQUEST_ID_CHARS + 1);
        for id in ["", "has space", "slash/id", "ünicode", too_long.as_str()] {
            assert!(ActiveRequest::start(id).is_err(), "{id:?}");
            assert!(cancel_request(id).is_err(), "{id:?}");
        }
        let longest = "x".repeat(MAX_REQUEST_ID_CHARS);
        assert!(validate_request_id(&longest).is_ok());
        assert!(validate_request_id("Req_1-a").is_ok());
    }

    #[test]
    fn cancel_before_start_prevents_the_generation() {
        let id = fresh();
        let unrelated = fresh();
        cancel_request(&id).unwrap();
        let refused = ActiveRequest::start(&id).err();
        assert_eq!(refused.as_deref(), Some(CANCELLED_BEFORE_START));
        assert!(refused.unwrap().starts_with("cancelled:"));
        let other = ActiveRequest::start(&unrelated).unwrap();
        assert!(!other.token.is_cancelled());
        // The tombstone is consumed by the start it refused.
        drop(ActiveRequest::start(&id).unwrap());
    }

    #[test]
    fn running_requests_cancel_immediately_refuse_duplicates_and_release_on_drop() {
        let id = fresh();
        let request = ActiveRequest::start(&id).unwrap();
        assert!(
            ActiveRequest::start(&id).is_err(),
            "duplicate ids are refused"
        );
        assert!(!request.token.is_cancelled());
        cancel_request(&id).unwrap();
        assert!(request.token.is_cancelled());
        drop(request);
        // Cancelling a running request leaves no tombstone: the ID is free.
        let again = ActiveRequest::start(&id).unwrap();
        assert!(!again.token.is_cancelled());
    }

    #[test]
    fn late_or_unknown_cancels_do_not_affect_new_requests() {
        let finished = fresh();
        drop(ActiveRequest::start(&finished).unwrap());
        cancel_request(&finished).unwrap();
        cancel_request(&fresh()).unwrap();
        let next = ActiveRequest::start(&fresh()).unwrap();
        assert!(!next.token.is_cancelled());
    }

    #[test]
    fn tombstones_expire_and_stay_bounded() {
        let registry = local();
        let t0 = Instant::now();
        let at = |secs: u64| t0 + Duration::from_secs(secs);
        let (expired, live) = (fresh(), fresh());
        lock(registry).cancel(&expired, at(0)).unwrap();
        lock(registry).cancel(&live, at(1)).unwrap();
        let start = |id: &str, secs| ActiveRequest::start_in(registry, id, at(secs));
        assert!(
            start(&expired, 90).is_ok(),
            "expired tombstones are ignored"
        );
        assert_eq!(
            start(&live, 90).err().as_deref(),
            Some(CANCELLED_BEFORE_START)
        );

        // At capacity the oldest tombstone is evicted, never the newest.
        let ids: Vec<String> = (0..MAX_TOMBSTONES + 3).map(|_| fresh()).collect();
        for (n, id) in ids.iter().enumerate() {
            lock(registry).cancel(id, at(100 + n as u64)).unwrap();
        }
        assert_eq!(lock(registry).cancelled.len(), MAX_TOMBSTONES);
        let newest = ids.last().unwrap();
        assert_eq!(
            start(newest, 170).err().as_deref(),
            Some(CANCELLED_BEFORE_START)
        );
        assert!(start(&ids[0], 170).is_ok(), "oldest tombstone was evicted");
        // Every surviving tombstone expires on the next prune.
        lock(registry).prune(at(100 + 90 + ids.len() as u64));
        assert!(lock(registry).cancelled.is_empty());
    }

    #[test]
    fn tombstones_never_occupy_active_slots() {
        let registry = local();
        let now = Instant::now();
        for _ in 0..MAX_TOMBSTONES {
            lock(registry).cancel(&fresh(), now).unwrap();
        }
        let running: Vec<ActiveRequest> = (0..MAX_ACTIVE_REQUESTS)
            .map(|_| ActiveRequest::start_in(registry, &fresh(), now).unwrap())
            .collect();
        let over = ActiveRequest::start_in(registry, &fresh(), now);
        assert!(over.err().unwrap().contains("Too many recaps"));
        // Dropping one lease frees exactly one slot.
        let mut running = running;
        running.pop();
        let replacement = ActiveRequest::start_in(registry, &fresh(), now).unwrap();
        assert!(!replacement.token.is_cancelled());
        assert_eq!(lock(registry).active.len(), MAX_ACTIVE_REQUESTS);
        drop((running, replacement));
        assert!(lock(registry).active.is_empty());
    }
}
