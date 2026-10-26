//! Shadow-mode observation of a NIP-FI WebSocket session whose upgrade passed.
//! The assertion lives only here: never on the connection, the issuer-scoped
//! identity registries, the admission gate, a terminal frame or cancellation.
//! A session records at most one admission verdict: a pairing or deny-set
//! would-deny where enforce checks them, the admit once the connection is
//! actually admitted, or `deadline` when enforce's deadline passes first. An
//! admitted session then records at most one end: `expired` at the deadline,
//! or `revoked` when a deny for its key or owner arrives once it registered.
//! Every transition runs under one session lock, and a session whose socket was
//! cancelled, or whose AUTH ended unadmitted for a non-NIP-FI reason, retires
//! without a record.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

use buzz_auth::{DenialClass, VerifiedAssertion};
use nostr::PublicKey;
use tokio_util::sync::CancellationToken;

use crate::nip_fi_core::NipFiSession;
use crate::nip_fi_shadow::{Stage, WouldDeny};
use crate::state::AppState;

#[derive(Clone, Copy)]
enum Phase {
    /// `revoked`: a deny for the key or owner arrived after registration, which
    /// enforce's registered-session scan acts on whatever the phase.
    Pending {
        revoked: bool,
    },
    Admitted,
    Ended,
}

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

    /// Records one `revoked` end for each registered session under `issuer`
    /// whose asserted key or verified owner is `pubkey`, matched as enforce's
    /// [`NipFiSession`] matches, at once if admitted or at its admit if still
    /// pending; a session already ended records nothing, so re-sent denies
    /// are idempotent.
    pub(crate) fn would_close(&self, issuer: &str, pubkey: &[u8]) {
        let live: Vec<_> = self
            .registered()
            .values()
            .filter_map(Weak::upgrade)
            .collect();
        for session in live {
            let assertion = &session.assertion;
            if assertion.asserted_key().is_some_and(|key| {
                NipFiSession::from_assertion(assertion).matches(issuer, &key.to_bytes(), pubkey)
            }) {
                session.revoke();
            }
        }
    }
}

/// A shadow session exists only for an assertion verified against a resolved
/// community, whose label all its records carry.
pub(crate) struct ShadowSession {
    id: u64,
    route: &'static str,
    community: metrics::SharedString,
    assertion: VerifiedAssertion,
    phase: Mutex<Phase>,
    /// The socket's cancellation, read only to stop recording.
    cancel: OnceLock<CancellationToken>,
    sessions: Arc<ShadowSessions>,
    expiry: OnceLock<tokio::task::AbortHandle>,
}

impl ShadowSession {
    /// Starts observing at the upgrade instant, arming a record-only timer at
    /// the deadline enforce would apply; dropping the session stops it.
    pub(crate) fn start(
        state: &AppState,
        route: &'static str,
        community: metrics::SharedString,
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
            community,
            assertion,
            phase: Mutex::new(Phase::Pending { revoked: false }),
            cancel: OnceLock::new(),
            sessions,
            expiry: OnceLock::new(),
        });
        let weak = Arc::downgrade(&session);
        let timer = tokio::spawn(async move {
            let remaining = (deadline - chrono::Utc::now()).to_std().unwrap_or_default();
            tokio::time::sleep(remaining).await;
            if let Some(session) = weak.upgrade() {
                session.expire();
            }
        });
        let _ = session.expiry.set(timer.abort_handle());
        session
    }

    /// Stops recording once `cancel` fires, checked inside every transition.
    pub(crate) fn fence(&self, cancel: CancellationToken) {
        let _ = self.cancel.set(cancel);
    }

    /// At enforce's key-pairing point: records the would-deny when the
    /// NIP-42 key is not the asserted one, or the assertion has no key.
    pub(crate) fn observe_pairing(&self, pubkey: PublicKey) {
        if self.assertion.asserted_key() != Some(pubkey) {
            self.decide(Err((Stage::Pairing, DenialClass::AuthorizationDenied)));
        }
    }

    /// At enforce's token-owner checks: records the would-deny when an agent
    /// assertion's owner is banned or differs from the agent's stored owner.
    pub(crate) async fn observe_owner(
        &self,
        state: &AppState,
        community: buzz_core::CommunityId,
        pubkey: PublicKey,
    ) {
        let Some(owner) = self.assertion.owner() else {
            return;
        };
        if let Some(class) =
            crate::handlers::auth::token_owner_denial(state, community, pubkey, owner).await
        {
            self.decide(Err((Stage::Owner, class)));
        }
    }

    /// At enforce's post-registration deny-set check: registers the session,
    /// so a later deny marks it, then records the deny-set would-deny.
    pub(crate) fn observe_deny_set(self: &Arc<Self>, state: &AppState) {
        self.sessions
            .registered()
            .insert(self.id, Arc::downgrade(self));
        if deny_listed(state, &self.assertion) {
            self.decide(Err((Stage::DenySet, DenialClass::AuthorizationDenied)));
        }
    }

    /// Where the connection is actually admitted: records the admit, then
    /// `revoked` if a deny marked the session since its deny-set check.
    pub(crate) fn admit(&self) {
        self.decide(Ok(()));
    }

    /// Enforce's deadline: the admission verdict while pending, else the end.
    /// A pending session a deny already marked records nothing, as enforce's
    /// scan closed it first.
    pub(crate) fn expire(&self) {
        self.transition(|phase| match *phase {
            Phase::Pending { revoked: false } => {
                *phase = Phase::Ended;
                let verdict = Err((Stage::Deadline, DenialClass::EvidenceRejected));
                crate::nip_fi_shadow::record_for(self.route, self.community.clone(), verdict);
            }
            Phase::Admitted => self.end(phase, "expired"),
            _ => {}
        });
    }

    /// Retires a pending session without a record: its connection was
    /// refused for a non-NIP-FI reason, so enforce would record nothing.
    pub(crate) fn retire_pending(&self) {
        self.transition(|phase| {
            if let Phase::Pending { .. } = phase {
                *phase = Phase::Ended;
            }
        });
    }

    fn revoke(&self) {
        self.transition(|phase| match phase {
            Phase::Pending { revoked } => *revoked = true,
            Phase::Admitted => self.end(phase, "revoked"),
            Phase::Ended => {}
        });
    }

    fn decide(&self, verdict: Result<(), WouldDeny>) {
        self.transition(|phase| {
            let Phase::Pending { revoked } = *phase else {
                return;
            };
            let admitted = verdict.is_ok();
            crate::nip_fi_shadow::record_for(self.route, self.community.clone(), verdict);
            *phase = if admitted {
                Phase::Admitted
            } else {
                Phase::Ended
            };
            if admitted && revoked {
                self.end(phase, "revoked");
            }
        });
    }

    /// Ends an admitted session.
    fn end(&self, phase: &mut Phase, reason: &'static str) {
        *phase = Phase::Ended;
        metrics::counter!(
            "buzz_nip_fi_shadow_session_end_total",
            "route" => self.route,
            "reason" => reason,
            "community" => self.community.clone()
        )
        .increment(1);
    }

    /// Runs one transition under the session lock, after retiring a session
    /// whose socket is already cancelled.
    fn transition(&self, step: impl FnOnce(&mut Phase)) {
        let mut phase = self.phase.lock().unwrap_or_else(PoisonError::into_inner);
        if self
            .cancel
            .get()
            .is_some_and(CancellationToken::is_cancelled)
        {
            *phase = Phase::Ended;
        }
        step(&mut phase);
    }
}

/// One AUTH attempt, audio's through its join commit: refused, or dropped,
/// while the session is still pending, it ended unadmitted for a non-NIP-FI
/// reason (NIP-42 failure, ordinary policy refusal), so the session retires
/// without a record.
pub(crate) struct AuthAttempt<'a>(pub(crate) Option<&'a Arc<ShadowSession>>);

impl AuthAttempt<'_> {
    /// Retires a pending session where the refusal is decided, before the
    /// refusal's frames and cleanup are awaited.
    pub(crate) fn refused(&self) {
        if let Some(session) = self.0 {
            session.retire_pending();
        }
    }
}

impl Drop for AuthAttempt<'_> {
    fn drop(&mut self) {
        self.refused();
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
    community: &metrics::SharedString,
    assertion: &VerifiedAssertion,
) -> bool {
    let denied = deny_listed(state, assertion);
    if denied {
        let verdict = Err((Stage::DenySet, DenialClass::AuthorizationDenied));
        crate::nip_fi_shadow::record_for("ws", community.clone(), verdict);
    }
    denied
}

fn deny_listed(state: &AppState, assertion: &VerifiedAssertion) -> bool {
    state
        .nip_fi_deny_map
        .as_deref()
        .is_some_and(|map| crate::nip_fi_core::assertion_denied(map, assertion, chrono::Utc::now()))
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
        collect(body, false, false)
    }

    /// [`shadow_records`], each prefixed with its `route` label.
    pub(crate) fn shadow_records_by_route(body: impl Future<Output = ()>) -> Vec<String> {
        collect(body, true, false)
    }

    /// [`shadow_records`] on paused Tokio time, which `tokio::time::advance`
    /// moves; a timer reads its delay from the wall clock when first polled.
    fn shadow_records_paused(body: impl Future<Output = ()>) -> Vec<String> {
        collect(body, false, true)
    }

    fn collect(body: impl Future<Output = ()>, by_route: bool, paused: bool) -> Vec<String> {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(paused)
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
            let record = match (by_route, label("route")) {
                (true, Some(route)) => format!("{route} {record}"),
                _ => record,
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
    /// `for_test` issuer. Its database is unreachable with a 100 ms acquire
    /// timeout, so a path that reaches Postgres fails fast whether or not
    /// one runs locally.
    pub(crate) async fn shadow_state(denied: Option<PublicKey>) -> AppState {
        let mut state =
            (*crate::state::tests::test_state_with_database_url("postgres://127.0.0.1:1/none")
                .await)
                .clone();
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

    pub(crate) fn session(
        state: &AppState,
        key: Option<PublicKey>,
        lifetime: Duration,
    ) -> Arc<ShadowSession> {
        let assertion = VerifiedAssertion::for_test(key, vec![Utc::now() + lifetime]);
        ShadowSession::start(
            state,
            "ws",
            crate::nip_fi_shadow::BOUND.into(),
            assertion,
            Utc::now(),
        )
    }

    fn auth(session: &Arc<ShadowSession>, state: &AppState, pubkey: PublicKey) {
        session.observe_pairing(pubkey);
        session.observe_deny_set(state);
        session.admit();
    }

    /// Lets every spawned timer read its deadline, then moves paused time
    /// past `ms` milliseconds.
    async fn advance_ms(ms: u64) {
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(ms)).await;
        tokio::task::yield_now().await;
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

    /// Delivers a real cross-pod disconnect for `key` to a shadow pod.
    pub(crate) fn deliver(state: &AppState, key: &PublicKey) -> buzz_auth::CrossPodMergeResult {
        let msg = buzz_pubsub::NipFiDisconnect {
            issuer: "test-issuer".to_owned(),
            pubkey_bytes: key.to_bytes().to_vec(),
            until_unix: (Utc::now() + Duration::minutes(5)).timestamp(),
            until_unix_nanos: 0,
        };
        match crate::api::nip_fi::apply_nip_fi_disconnect(state, &msg, Utc::now()) {
            crate::api::nip_fi::NipFiDisconnectApplyResult::Applied(merged) => merged,
            other => panic!("disconnect not applied: {other:?}"),
        }
    }

    /// An agent assertion for `agent` acting for `owner`, valid for an hour.
    pub(crate) fn owned_assertion(agent: PublicKey, owner: PublicKey) -> VerifiedAssertion {
        VerifiedAssertion::for_test(Some(agent), vec![Utc::now() + Duration::hours(1)])
            .with_owner_for_test(owner)
    }

    // An owner's disconnect, delivered cross-pod after admission or between
    // the deny-set check and the admit, ends each of its agents' ws and audio
    // sessions `revoked` exactly once and closes no real socket, even one the
    // real registry would match; the owner under another issuer and an
    // ownerless session of the agent key take no end. Mutation: dropping the
    // owner arm of `would_close` loses every `end/revoked`.
    #[test]
    fn owner_disconnect_ends_owned_agent_sessions_and_closes_nothing() {
        let records = shadow_records_by_route(async {
            let state = pod(8).await;
            let (agent, owner) = (Keys::generate().public_key(), Keys::generate().public_key());
            let owned = |route| {
                let assertion = owned_assertion(agent, owner);
                ShadowSession::start(
                    &state,
                    route,
                    crate::nip_fi_shadow::BOUND.into(),
                    assertion,
                    Utc::now(),
                )
            };
            let cancel = CancellationToken::new();
            let control = crate::state::CommunityConnectionControl::new(cancel.clone());
            let (terminal_tx, mut terminal_rx) = tokio::sync::mpsc::channel(1);
            control.set_terminal_frame_sender(terminal_tx);
            control.set_proven_identity(
                agent.to_bytes().to_vec(),
                Some(NipFiSession::from_assertion(&owned_assertion(agent, owner))),
            );
            let community = buzz_core::tenant::CommunityId::from_uuid(uuid::Uuid::new_v4());
            let _guard =
                state
                    .community_connections
                    .register(uuid::Uuid::new_v4(), community, control);

            let admitted = [owned("ws"), owned("audio")];
            let pending = [owned("ws"), owned("audio")];
            let ownerless = session(&state, Some(agent), Duration::hours(1));
            for s in admitted.iter().chain([&ownerless]) {
                auth(s, &state, agent);
            }
            for s in &pending {
                s.observe_pairing(agent);
                s.observe_deny_set(&state);
            }
            state
                .nip_fi_shadow_sessions
                .would_close("other-issuer", &owner.to_bytes());
            for s in &admitted {
                assert!(matches!(*s.phase.lock().unwrap(), Phase::Admitted));
            }
            for s in &pending {
                let phase = s.phase.lock().unwrap();
                assert!(matches!(*phase, Phase::Pending { revoked: false }));
            }
            deliver(&state, &owner);
            revoke(&state, &owner);
            for s in &pending {
                s.admit();
            }
            assert!(!cancel.is_cancelled(), "shadow never cancels a socket");
            assert!(terminal_rx.try_recv().is_err(), "shadow sends no frame");
        });
        let expected = [
            "audio admit/admit",
            "audio admit/admit",
            "audio end/revoked",
            "audio end/revoked",
            "ws admit/admit",
            "ws admit/admit",
            "ws admit/admit",
            "ws end/revoked",
            "ws end/revoked",
        ];
        assert_eq!(records, expected);
    }

    // The shadow deny observer reads the owner's deny entry at the upgrade
    // and after registration. Mutation: dropping the owner arm of
    // `assertion_denied` records two admits instead.
    #[test]
    fn owner_deny_entry_records_deny_set_at_upgrade_and_after_registration() {
        let (agent, owner) = (Keys::generate().public_key(), Keys::generate().public_key());
        let records = shadow_records(async move {
            let state = shadow_state(Some(owner)).await;
            let assertion = owned_assertion(agent, owner);
            assert!(upgrade_denied(
                &state,
                &crate::nip_fi_shadow::BOUND.into(),
                &assertion
            ));
            let s = ShadowSession::start(
                &state,
                "ws",
                crate::nip_fi_shadow::BOUND.into(),
                assertion,
                Utc::now(),
            );
            auth(&s, &state, agent);
        });
        assert_eq!(records, ["deny_set/denied", "deny_set/denied"]);
    }

    /// A shadow pod whose deny map holds `capacity` entries for the
    /// `for_test` issuer.
    pub(crate) async fn pod(capacity: usize) -> AppState {
        let mut state = shadow_state(None).await;
        let mut config = (*state.config).clone();
        config.nip_fi.mode = buzz_auth::NipFiMode::Shadow;
        use buzz_auth::{FreshnessClass, IssuerPolicy, JwksSourceContract, TokenClass};
        let jwks = JwksSourceContract::new("https://idp.test/jwks.json".into(), 300, 86400);
        let policy = IssuerPolicy::new(
            "test-issuer".to_owned(),
            vec!["https://relay.test".to_owned()],
            TokenClass::DedicatedNipFi,
            FreshnessClass::OfflineJwt,
            vec![jsonwebtoken::Algorithm::ES256],
            30,
            3600,
            None,
            jwks.unwrap(),
        );
        config.nip_fi.registry.insert(policy.unwrap());
        state.config = Arc::new(config);
        let capacity = vec![IssuerCapacity {
            issuer: "test-issuer".to_owned(),
            capacity,
        }];
        state.nip_fi_deny_map = Some(Arc::new(NipFiDenyMap::new(8, capacity)));
        state
    }

    // A deny landing between the clean deny-set check and the admit,
    // whether merged or refused for capacity, ends the admitted session
    // `revoked`, as enforce's registered-session scan closes it in any phase;
    // expiry contending on either side of the admit cannot take that end.
    // Mutation: dropping the revocation mark, or letting a marked pending
    // session expire, loses the `revoked` end.
    #[test]
    fn deny_between_map_read_and_admit_records_admit_then_revoked() {
        let records = shadow_records(async {
            for capacity in [8, 1] {
                let state = pod(capacity).await;
                let key = Keys::generate().public_key();
                if capacity == 1 {
                    deliver(&state, &Keys::generate().public_key());
                }
                let s = session(&state, Some(key), Duration::hours(1));
                s.observe_pairing(key);
                s.observe_deny_set(&state);
                let merged = deliver(&state, &key);
                assert_eq!(
                    merged == buzz_auth::CrossPodMergeResult::Merged,
                    capacity == 8
                );
                s.expire();
                s.admit();
                s.expire();
            }
        });
        let both = ["admit/admit", "admit/admit", "end/revoked", "end/revoked"];
        assert_eq!(records, both);
    }

    // Expiry before AUTH is the session's admission verdict, so AUTH records
    // nothing; expiry after admission ends it once, so a later deny records
    // nothing.
    #[test]
    fn deadline_records_one_expired_end_before_or_after_admission() {
        let records = shadow_records_paused(async {
            let state = shadow_state(None).await;
            let key = Keys::generate().public_key();
            let early = session(&state, Some(key), Duration::seconds(10));
            let late = session(&state, Some(key), Duration::seconds(30));
            advance_ms(20_000).await;
            auth(&early, &state, key);
            auth(&late, &state, key);
            advance_ms(20_000).await;
            revoke(&state, &key);
        });
        assert_eq!(records, ["admit/admit", "deadline/rejected", "end/expired"]);
    }

    // The deadline passing while the socket is still bootstrapping, before
    // AUTH, records one `deadline` verdict and no end, and the observer
    // cancels nothing.
    #[test]
    fn deadline_during_bootstrap_records_one_verdict_and_cancels_nothing() {
        let records = shadow_records_paused(async {
            let state = shadow_state(None).await;
            let key = Keys::generate().public_key();
            let s = session(&state, Some(key), Duration::seconds(10));
            let cancel = tokio_util::sync::CancellationToken::new();
            let control = crate::state::CommunityConnectionControl::new(cancel.clone());
            control.attach_nip_fi_shadow(Some(Arc::clone(&s)));
            advance_ms(20_000).await;
            assert!(!cancel.is_cancelled(), "shadow never cancels the socket");
            auth(&s, &state, key);
            revoke(&state, &key);
        });
        assert_eq!(records, ["deadline/rejected"]);
    }

    // A connection closed before its deadline records no end. Mutation: not
    // unregistering on drop leaves a stale registry entry.
    #[test]
    fn dropped_session_records_no_end() {
        let records = shadow_records_paused(async {
            let state = shadow_state(None).await;
            let key = Keys::generate().public_key();
            let s = session(&state, Some(key), Duration::seconds(10));
            auth(&s, &state, key);
            let weak = Arc::downgrade(&s);
            drop(s);
            assert!(weak.upgrade().is_none());
            assert!(state.nip_fi_shadow_sessions.registered().is_empty());
            advance_ms(20_000).await;
        });
        assert_eq!(records, ["admit/admit"]);
    }

    // Once the socket is cancelled, no transition records, even while the
    // connection still holds references and before anything else runs: an
    // admitted session takes no `revoked` end from an immediate real deny,
    // and a pending one records no verdict or deadline. Mutation: skipping
    // the cancellation check in `transition` records both.
    #[test]
    fn cancelled_socket_records_nothing_without_yielding() {
        let records = shadow_records(async {
            let state = pod(8).await;
            let key = Keys::generate().public_key();
            let watch = |s: &Arc<ShadowSession>| {
                let cancel = tokio_util::sync::CancellationToken::new();
                let control = crate::state::CommunityConnectionControl::new(cancel.clone());
                control.attach_nip_fi_shadow(Some(Arc::clone(s)));
                (cancel, control)
            };
            let admitted = session(&state, Some(key), Duration::hours(1));
            let (cancel, _control) = watch(&admitted);
            auth(&admitted, &state, key);
            cancel.cancel();
            deliver(&state, &key);

            let pending = session(&state, Some(key), Duration::hours(1));
            let (cancel, _control) = watch(&pending);
            cancel.cancel();
            pending.observe_pairing(Keys::generate().public_key());
            pending.expire();
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
            assert!(upgrade_denied(
                &state,
                &crate::nip_fi_shadow::BOUND.into(),
                &listed
            ));
            assert!(!upgrade_denied(
                &state,
                &crate::nip_fi_shadow::BOUND.into(),
                &clean
            ));
        });
        assert_eq!(records, ["deny_set/denied"]);
    }
}
