//! Per-relay admission for local agent pairs.
//!
//! Removing a community from this device stops every local pair on its relay,
//! but a start already in flight can await (mesh preflight, a restart's stop
//! phase), let that stop sweep finish, then resume and connect to the removed
//! relay. So every spawn site captures an [`AdmissionSnapshot`] before its
//! async work and re-checks it with [`RelayAdmissions::admit`] while holding
//! `managed_agent_runtime_transition` — the lock that owns this table — until
//! the pair is registered, where a removal's stop sweep will find it.
//! `spawn_agent_child` requires the [`Admitted`] proof `admit` returns, so no
//! spawn site can skip the check.

use std::collections::HashMap;

use crate::app_state::AppState;

/// Refusal for a start whose relay was removed from this device. Callers that
/// start pairs in the background treat it as a quiet no-op, not a failure.
pub const RELAY_REMOVED_ERROR: &str = "relay was removed from this device";

/// Admission state per canonical relay URL (`buzz_core::relay::normalize_relay_url`).
#[derive(Default)]
pub struct RelayAdmissions(HashMap<String, RelayAdmission>);

#[derive(Clone, Copy, Default)]
struct RelayAdmission {
    removed: bool,
    /// Bumped on every removal, so work captured before a removal is never
    /// admitted again, even after a re-add.
    epoch: u64,
}

/// Proof that a relay passed [`RelayAdmissions::admit`]. It borrows the table,
/// so it cannot outlive the transition lock it was checked under.
pub struct Admitted<'a> {
    relay: String,
    _table: std::marker::PhantomData<&'a RelayAdmissions>,
}

impl Admitted<'_> {
    /// Refuses a spawn on any relay other than the one admitted.
    pub fn covers(&self, relay_url: &str) -> Result<(), String> {
        if canonical(relay_url)? != self.relay {
            return Err(format!("{relay_url} was not admitted for this spawn"));
        }
        Ok(())
    }
}

/// Relay epochs captured by a start before its async work.
#[derive(Clone, Debug, Default)]
pub struct AdmissionSnapshot(HashMap<String, u64>);

fn canonical(relay_url: &str) -> Result<String, String> {
    buzz_core_pkg::relay::normalize_relay_url(relay_url).map_err(|error| error.to_string())
}

impl RelayAdmissions {
    pub fn snapshot(&self) -> AdmissionSnapshot {
        AdmissionSnapshot(
            self.0
                .iter()
                .map(|(relay, admission)| (relay.clone(), admission.epoch))
                .collect(),
        )
    }

    /// Refuse this relay's pairs until it is re-added, and invalidate every
    /// snapshot taken so far.
    pub fn remove(&mut self, relay_url: &str) -> Result<(), String> {
        let admission = self.0.entry(canonical(relay_url)?).or_default();
        admission.removed = true;
        admission.epoch += 1;
        Ok(())
    }

    /// Admit a removed relay again. Work captured before the removal stays
    /// refused: its epoch is older.
    pub fn readd(&mut self, relay_url: &str) -> Result<(), String> {
        if let Some(admission) = self.0.get_mut(&canonical(relay_url)?) {
            admission.removed = false;
        }
        Ok(())
    }

    /// `Err(RELAY_REMOVED_ERROR)` if the relay is removed, or was removed
    /// after `snapshot` was captured.
    pub fn admit(
        &self,
        snapshot: &AdmissionSnapshot,
        relay_url: &str,
    ) -> Result<Admitted<'_>, String> {
        let relay = canonical(relay_url)?;
        let current = self.0.get(&relay).copied().unwrap_or_default();
        let captured = snapshot.0.get(&relay).copied().unwrap_or_default();
        if current.removed || current.epoch != captured {
            return Err(RELAY_REMOVED_ERROR.into());
        }
        Ok(Admitted {
            relay,
            _table: std::marker::PhantomData,
        })
    }
}

impl AdmissionSnapshot {
    /// Capture before any async work. Takes the transition lock briefly, so
    /// never call it while holding that lock. A poisoned lock still yields
    /// the table: capture must not be the reason a start skips its refusal.
    pub fn capture(state: &AppState) -> Self {
        state
            .managed_agent_runtime_transition
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .snapshot()
    }
}

/// Record that a community's relay was removed from this device. The caller
/// applies the shared-relay guard: only when no other saved community uses it.
/// Runs under the transition lock, so a start that already passed admission
/// has registered its pair before this returns and the caller's stop sweep
/// finds it.
pub fn remove_relay(state: &AppState, relay_url: &str) -> Result<(), String> {
    state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|error| error.to_string())?
        .remove(relay_url)
}

/// Record that a saved community on this relay was explicitly added again.
pub fn readd_relay(state: &AppState, relay_url: &str) -> Result<(), String> {
    state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|error| error.to_string())?
        .readd(relay_url)
}

#[cfg(test)]
mod tests {
    use super::{AdmissionSnapshot, RelayAdmissions, RELAY_REMOVED_ERROR};

    const RELAY: &str = "wss://relay.example";

    fn refusal(
        admissions: &RelayAdmissions,
        snapshot: &AdmissionSnapshot,
        relay: &str,
    ) -> Result<(), String> {
        admissions.admit(snapshot, relay).map(|_| ())
    }

    #[test]
    fn admission_covers_only_its_own_relay() {
        let admissions = RelayAdmissions::default();
        let admitted = admissions.admit(&admissions.snapshot(), RELAY).unwrap();
        assert_eq!(admitted.covers("WSS://relay.example/"), Ok(()));
        assert!(admitted.covers("wss://other.example").is_err());
    }

    #[test]
    fn removal_refuses_captured_and_fresh_work_until_readded() {
        let mut admissions = RelayAdmissions::default();
        let before = admissions.snapshot();
        admissions.remove("WSS://Relay.Example/").unwrap();

        assert_eq!(
            refusal(&admissions, &before, RELAY),
            Err(RELAY_REMOVED_ERROR.into())
        );
        assert_eq!(
            refusal(&admissions, &admissions.snapshot(), RELAY),
            Err(RELAY_REMOVED_ERROR.into())
        );
        admissions.readd(RELAY).unwrap();
        assert_eq!(refusal(&admissions, &admissions.snapshot(), RELAY), Ok(()));
    }

    #[test]
    fn work_captured_before_a_removal_stays_refused_after_readd() {
        let mut admissions = RelayAdmissions::default();
        let before_removal = admissions.snapshot();
        admissions.remove(RELAY).unwrap();
        admissions.readd(RELAY).unwrap();

        assert_eq!(
            refusal(&admissions, &before_removal, RELAY),
            Err(RELAY_REMOVED_ERROR.into())
        );
    }

    #[test]
    fn removal_leaves_other_relays_admitted() {
        let mut admissions = RelayAdmissions::default();
        let before = admissions.snapshot();
        admissions.remove(RELAY).unwrap();
        assert_eq!(refusal(&admissions, &before, "wss://other.example"), Ok(()));
    }
}
