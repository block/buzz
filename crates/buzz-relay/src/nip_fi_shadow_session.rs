//! Shadow-mode observation of a NIP-FI WebSocket session whose upgrade passed.
//! The assertion lives only here: never on the connection, the issuer-scoped
//! identity registries, the admission gate, a terminal frame or cancellation.
//! A session records at most one admission verdict, at its first successful
//! NIP-42 AUTH, and at most one session end: `expired` when enforce's deadline
//! passes, or `revoked` when a deny arrives for its key after admission.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

use axum::http::HeaderMap;
use buzz_auth::{DenialClass, VerifiedAssertion};
use nostr::PublicKey;

use crate::nip_fi_shadow::{Stage, WouldDeny};
use crate::state::AppState;

const PENDING: u8 = 0;
const ADMITTED: u8 = 1;
const ENDED: u8 = 2;

/// Sessions that reached the deny-set check, matched by a shadow disconnect.
/// Separate from every registry enforce's close scan reads.
#[derive(Default)]
pub(crate) struct ShadowSessions {
    next_id: AtomicU64,
    registered: Mutex<HashMap<u64, Weak<ShadowSession>>>,
}

impl ShadowSessions {
    fn registered(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Weak<ShadowSession>>> {
        self.registered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Records one `revoked` end for each admitted session of `pubkey` under
    /// `issuer`; a session already ended records nothing, so re-sent denies
    /// are idempotent.
    pub(crate) fn would_close(&self, issuer: &str, pubkey: &[u8]) {
        let live: Vec<_> = self
            .registered()
            .values()
            .filter_map(Weak::upgrade)
            .collect();
        for session in live {
            let key = session.assertion.asserted_key().map(|k| k.to_bytes());
            if session.assertion.identity().issuer() == issuer
                && key.as_ref().map(|k| &k[..]) == Some(pubkey)
            {
                session.end("revoked", false);
            }
        }
    }
}

pub(crate) struct ShadowSession {
    id: u64,
    route: &'static str,
    community: String,
    assertion: VerifiedAssertion,
    phase: AtomicU8,
    sessions: Arc<ShadowSessions>,
    expiry: OnceLock<tokio::task::AbortHandle>,
}

impl ShadowSession {
    /// Starts observing at the upgrade instant, arming a record-only timer at
    /// the deadline enforce would apply; dropping the session stops it.
    pub(crate) fn start(
        state: &AppState,
        route: &'static str,
        headers: &HeaderMap,
        assertion: VerifiedAssertion,
        connection_time: chrono::DateTime<chrono::Utc>,
    ) -> Arc<Self> {
        let nip_fi = &state.config.nip_fi;
        let deadline = crate::connection::compute_session_deadline(
            &assertion,
            connection_time,
            nip_fi.max_connection_lifetime(),
        );
        let sessions = Arc::clone(&state.nip_fi_shadow_sessions);
        let session = Arc::new(Self {
            id: sessions.next_id.fetch_add(1, Ordering::Relaxed),
            route,
            community: crate::nip_fi_shadow::community_label(headers, &nip_fi.communities),
            assertion,
            phase: AtomicU8::new(PENDING),
            sessions,
            expiry: OnceLock::new(),
        });
        let weak = Arc::downgrade(&session);
        let timer = tokio::spawn(async move {
            let remaining = (deadline - chrono::Utc::now()).to_std().unwrap_or_default();
            tokio::time::sleep(remaining).await;
            if let Some(session) = weak.upgrade() {
                session.end("expired", true);
            }
        });
        let _ = session.expiry.set(timer.abort_handle());
        session
    }

    /// At enforce's key-pairing point: records the would-deny when the
    /// NIP-42 key is not the asserted one, or the assertion has no key.
    pub(crate) fn observe_pairing(&self, pubkey: PublicKey) {
        if self.assertion.asserted_key() != Some(pubkey) {
            self.decide(Err((Stage::Pairing, DenialClass::AuthorizationDenied)));
        }
    }

    /// At enforce's post-registration deny-set check: registers the session,
    /// then records the deny-set would-deny or the admit. A disconnect racing
    /// this either finds the registered session or left its deny entry.
    pub(crate) fn observe_admission(self: &Arc<Self>, state: &AppState) {
        if self.phase.load(Ordering::Acquire) != PENDING {
            return;
        }
        self.sessions
            .registered()
            .insert(self.id, Arc::downgrade(self));
        self.decide(if deny_listed(state, &self.assertion) {
            Err((Stage::DenySet, DenialClass::AuthorizationDenied))
        } else {
            Ok(())
        });
    }

    fn decide(&self, verdict: Result<(), WouldDeny>) {
        let next = if verdict.is_ok() { ADMITTED } else { ENDED };
        if self.advance(PENDING, next) {
            crate::nip_fi_shadow::record_for(self.route, self.community.clone(), verdict);
        }
    }

    /// Ends an admitted session, or a pending one when `from_pending`.
    fn end(&self, reason: &'static str, from_pending: bool) {
        let live = |phase| phase == ADMITTED || (from_pending && phase == PENDING);
        let ended = self
            .phase
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |p| {
                live(p).then_some(ENDED)
            });
        if ended.is_ok() {
            metrics::counter!(
                "buzz_nip_fi_shadow_session_end_total",
                "route" => self.route,
                "reason" => reason,
                "community" => self.community.clone()
            )
            .increment(1);
        }
    }

    fn advance(&self, from: u8, to: u8) -> bool {
        self.phase
            .compare_exchange(from, to, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

impl Drop for ShadowSession {
    fn drop(&mut self) {
        if let Some(timer) = self.expiry.get() {
            timer.abort();
        }
        self.sessions.registered().remove(&self.id);
    }
}

/// Root's upgrade-time deny-map bounce, observed: records the would-deny and
/// returns `true` when enforce would have refused the upgrade here.
pub(crate) fn upgrade_denied(
    state: &AppState,
    headers: &HeaderMap,
    assertion: &VerifiedAssertion,
) -> bool {
    let denied = deny_listed(state, assertion);
    if denied {
        let verdict = Err((Stage::DenySet, DenialClass::AuthorizationDenied));
        crate::nip_fi_shadow::record("ws", headers, &state.config.nip_fi.communities, verdict);
    }
    denied
}

fn deny_listed(state: &AppState, assertion: &VerifiedAssertion) -> bool {
    let issuer = assertion.identity().issuer();
    assertion
        .asserted_key()
        .zip(state.nip_fi_deny_map.as_deref())
        .is_some_and(|(key, map)| map.is_denied(issuer, &key, chrono::Utc::now()))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::future::Future;

    use buzz_auth::{IssuerCapacity, NipFiDenyMap};
    use chrono::{Duration, Utc};
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};
    use nostr::Keys;

    use super::*;

    /// Runs `body` on a current-thread runtime, returning every shadow
    /// admission record as `stage/outcome` and every session end as
    /// `end/reason`, sorted.
    pub(crate) fn shadow_records(body: impl Future<Output = ()>) -> Vec<String> {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        metrics::with_local_recorder(&recorder, || rt.block_on(body));
        let mut out = Vec::new();
        for (key, _, _, value) in snapshotter.snapshot().into_vec() {
            let key = key.key();
            let label = |name| {
                key.labels()
                    .find(|l| l.key() == name)
                    .map(|l| l.value().to_owned())
            };
            let record = match key.name() {
                "buzz_nip_fi_shadow_total" => {
                    format!("{}/{}", label("stage").unwrap(), label("outcome").unwrap())
                }
                "buzz_nip_fi_shadow_session_end_total" => {
                    format!("end/{}", label("reason").unwrap())
                }
                _ => continue,
            };
            let DebugValue::Counter(n) = value else {
                unreachable!()
            };
            out.extend(std::iter::repeat_n(record, n as usize));
        }
        out.sort();
        out
    }

    /// A shadow test state whose deny map lists `denied` under the
    /// `for_test` issuer.
    pub(crate) async fn shadow_state(denied: Option<PublicKey>) -> AppState {
        let mut state = (*crate::state::tests::test_state().await).clone();
        let capacity = vec![IssuerCapacity {
            issuer: "test-issuer".to_owned(),
            capacity: 8,
        }];
        let map = NipFiDenyMap::new(8, capacity);
        if let Some(key) = denied {
            map.merge_cross_pod_deny(
                "test-issuer",
                &key,
                Utc::now() + Duration::hours(1),
                Utc::now(),
            );
        }
        state.nip_fi_deny_map = Some(Arc::new(map));
        state.nip_fi_shadow_sessions = Arc::default();
        state
    }

    fn session(state: &AppState, key: Option<PublicKey>, lifetime: Duration) -> Arc<ShadowSession> {
        let assertion = VerifiedAssertion::for_test(key, vec![Utc::now() + lifetime]);
        ShadowSession::start(state, "ws", &HeaderMap::new(), assertion, Utc::now())
    }

    fn auth(session: &Arc<ShadowSession>, state: &AppState, pubkey: PublicKey) {
        session.observe_pairing(pubkey);
        session.observe_admission(state);
    }

    fn revoke(state: &AppState, key: &PublicKey) {
        state
            .nip_fi_shadow_sessions
            .would_close("test-issuer", &key.to_bytes());
    }

    // Mutation: recording on a repeated AUTH, or letting the re-sent deny
    // end the session again, adds a record.
    #[test]
    fn admitted_session_records_one_admit_and_one_revoked_end() {
        let records = shadow_records(async {
            let state = shadow_state(None).await;
            let key = Keys::generate().public_key();
            let s = session(&state, Some(key), Duration::hours(1));
            auth(&s, &state, key);
            auth(&s, &state, key);
            revoke(&state, &Keys::generate().public_key());
            state
                .nip_fi_shadow_sessions
                .would_close("other-issuer", &key.to_bytes());
            revoke(&state, &key);
            revoke(&state, &key);
        });
        assert_eq!(records, ["admit/admit", "end/revoked"]);
    }

    // Mismatched and claimless keys are both pairing denials; nothing after a
    // denial records, because enforce would have closed the connection.
    #[test]
    fn pairing_denial_is_the_only_record() {
        let records = shadow_records(async {
            let state = shadow_state(None).await;
            let key = Keys::generate().public_key();
            for asserted in [Some(Keys::generate().public_key()), None] {
                let s = session(&state, asserted, Duration::hours(1));
                auth(&s, &state, key);
                auth(&s, &state, asserted.unwrap_or(key));
                revoke(&state, &key);
            }
        });
        assert_eq!(records, ["pairing/denied", "pairing/denied"]);
    }

    // Mutation: checking the deny map before registering, or not at all,
    // records an admit.
    #[test]
    fn deny_listed_key_records_deny_set_and_no_revoked_end() {
        let key = Keys::generate().public_key();
        let records = shadow_records(async move {
            let state = shadow_state(Some(key)).await;
            let s = session(&state, Some(key), Duration::hours(1));
            auth(&s, &state, key);
            revoke(&state, &key);
        });
        assert_eq!(records, ["deny_set/denied"]);
    }

    // Expiry before AUTH ends the session, so AUTH records no verdict; expiry
    // after admission ends it once, so a later deny records nothing.
    #[test]
    fn deadline_records_one_expired_end_before_or_after_admission() {
        let records = shadow_records(async {
            let state = shadow_state(None).await;
            let key = Keys::generate().public_key();
            let early = session(&state, Some(key), Duration::milliseconds(20));
            let late = session(&state, Some(key), Duration::milliseconds(40));
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            auth(&early, &state, key);
            auth(&late, &state, key);
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            revoke(&state, &key);
        });
        assert_eq!(records, ["admit/admit", "end/expired", "end/expired"]);
    }

    // A connection closed before its deadline records no end. Mutation: not
    // unregistering on drop leaves a stale registry entry.
    #[test]
    fn dropped_session_records_no_end() {
        let records = shadow_records(async {
            let state = shadow_state(None).await;
            let key = Keys::generate().public_key();
            let s = session(&state, Some(key), Duration::milliseconds(20));
            auth(&s, &state, key);
            let weak = Arc::downgrade(&s);
            drop(s);
            assert!(weak.upgrade().is_none());
            assert!(state.nip_fi_shadow_sessions.registered().is_empty());
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        });
        assert_eq!(records, ["admit/admit"]);
    }

    #[test]
    fn upgrade_deny_map_hit_records_and_reports_denied() {
        let key = Keys::generate().public_key();
        let records = shadow_records(async move {
            let state = shadow_state(Some(key)).await;
            let listed =
                VerifiedAssertion::for_test(Some(key), vec![Utc::now() + Duration::hours(1)]);
            let clean =
                VerifiedAssertion::for_test(Some(Keys::generate().public_key()), vec![Utc::now()]);
            assert!(upgrade_denied(&state, &HeaderMap::new(), &listed));
            assert!(!upgrade_denied(&state, &HeaderMap::new(), &clean));
        });
        assert_eq!(records, ["deny_set/denied"]);
    }
}
