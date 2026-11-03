//! Shadow-mode recording of what enforce would have decided. The verdict
//! never reaches admission, a connection, or a response. Shadow resolves the
//! community exactly as enforce does, database lookup and freshness included,
//! and once per HTTP request. Labels are bounded and never the raw `Host`: the
//! community label is the configured URI for the `env` source (a set bounded
//! by config), `bound` for the `database` source, and `unmapped` when the
//! community did not resolve.
//!
//! Each would-deny also writes one info line, `nip-fi shadow would-deny`,
//! carrying the metric's labels plus two fields kept off the metric so its
//! cardinality stays bounded: `reason`, one value from a fixed snake_case set
//! naming the private condition the public `outcome` class collapses, and
//! `pubkey`, the hex Nostr key of the would-be-refused client where the step
//! knows one, else `absent`. A router-guard or upgrade would-deny precedes
//! any proof, so its line waits for the key the client later proves: the
//! request's verified NIP-98 key, or the connection's first verified NIP-42
//! AUTH key. It is written once, as soon as that key is proven, or with
//! `absent` when the request ends or the connection stops reading without
//! one; its counter increments at once. An admit writes no line. No token, assertion,
//! claim other than the public Nostr key, or issuer is logged.
//! [FI-TRACE-PRIVACY-NONPUBLIC]
//!
//! # Metrics
//!
//! | Series | Labels | One increment per |
//! |---|---|---|
//! | `buzz_nip_fi_shadow_total` | `route` (`http`, `ws`, `audio`), `stage`, `outcome`, `community` | NIP-FI decision enforce would make: one per HTTP request that reaches admission, one per WebSocket upgrade enforce would refuse, and one per observed session: at the AUTH where enforce would admit or deny it, or `deadline` when its deadline passes first. A request or upgrade whose community binding goes stale before enforce's final re-check also records `deadline`. An agent assertion enforce would refuse for its token owner, banned or not the agent's stored owner, records `owner` |
//! | `buzz_nip_fi_shadow_strict_proof_total` | `route` (`bridge`, `blossom`), `outcome` (`pass`, `rejected`), `community` | strict NIP-98 side check on a route whose proof is stricter in enforce (payload tag, Blossom proof); not every request runs one |
//! | `buzz_nip_fi_shadow_session_end_total` | `route` (`ws`, `audio`), `reason` (`expired`, `revoked`), `community` | admitted shadow session enforce would have ended; a session never admitted records none |
//! | `buzz_nip_fi_shadow_disconnect_total` | `route` (`admin`, `cross_pod`), `outcome` | disconnect-path event that moves a real disconnect counter in enforce: an accepted admin command, a capacity rejection, a subscriber lag, or a cross-pod capacity or poison failsafe. An admin command refused for its header, body, proof, replay, or a dependency error records nothing |
//!
//! Denominators differ. A would-deny rate is the non-`admit` share of
//! `buzz_nip_fi_shadow_total` for a route. A strict-proof rejection rate is
//! `rejected` over `pass + rejected` of its own series, never over
//! `buzz_nip_fi_shadow_total`: the side check runs once per proof, not once
//! per admission verdict. A session-end rate divides by the `admit` count of
//! `buzz_nip_fi_shadow_total` for the same `ws` or `audio` route.

use axum::http::HeaderMap;
use buzz_auth::DenialClass;

use metrics::SharedString;
use nostr::PublicKey;

use crate::nip_fi_core::AssertionRejection;
use crate::nip_fi_http::Resolution;

/// Which enforce step would have denied, with which public class, the private
/// reason behind it, and the client's key when the step knows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WouldDeny {
    pub(crate) stage: Stage,
    pub(crate) class: DenialClass,
    reason: &'static str,
    pubkey: Option<PublicKey>,
}

impl WouldDeny {
    /// A would-deny whose client key is not known at `stage`.
    pub(crate) const fn new(stage: Stage, class: DenialClass, reason: &'static str) -> Self {
        Self {
            stage,
            class,
            reason,
            pubkey: None,
        }
    }

    /// The same would-deny, attributed to `pubkey`.
    pub(crate) const fn by(self, pubkey: Option<PublicKey>) -> Self {
        Self { pubkey, ..self }
    }

    /// The assertion step's would-deny for `rejection`.
    pub(crate) fn assertion(rejection: AssertionRejection) -> Self {
        let reason = match rejection {
            AssertionRejection::Transport(DenialClass::MissingEvidence) => "missing_assertion",
            AssertionRejection::Transport(_) => "malformed_assertion",
            AssertionRejection::VerifierUnavailable => "verifier_unavailable",
            AssertionRejection::Verifier(e) => {
                let code = e.code();
                code.strip_prefix("nip_fi_").unwrap_or(code)
            }
        };
        Self::new(Stage::Assertion, rejection.denial_class(), reason)
    }
}

/// The enforce step a would-deny stops at: the shadow `stage` label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    Community,
    Cardinality,
    Nip98,
    Assertion,
    Pairing,
    DenySet,
    Deadline,
    Owner,
}

impl Stage {
    const fn label(self) -> &'static str {
        match self {
            Self::Community => "community",
            Self::Cardinality => "cardinality",
            Self::Nip98 => "nip98",
            Self::Assertion => "assertion",
            Self::Pairing => "pairing",
            Self::DenySet => "deny_set",
            Self::Deadline => "deadline",
            Self::Owner => "owner",
        }
    }
}

/// The `community` label of a `database`-source community that resolved.
pub(crate) const BOUND: &str = "bound";
/// The `community` label of a verdict whose community did not resolve.
const UNMAPPED: &str = "unmapped";
/// The reason for a Host whose community resolution failed: it maps to no
/// community, or the lookup itself was unavailable.
pub(crate) const UNRESOLVED: &str = "community_unresolved";
/// The deny-set step's would-deny, before attribution.
pub(crate) const DENY_SET: WouldDeny =
    WouldDeny::new(Stage::DenySet, DenialClass::AuthorizationDenied, "deny_set");
/// The reason for a community binding that went stale before admission.
pub(crate) const STALE: &str = "community_stale";

/// The `community` label of `community`.
pub(crate) fn label(community: &Resolution) -> SharedString {
    community
        .as_ref()
        .map_or(SharedString::const_str(UNMAPPED), |c| c.shadow_label())
}

/// Record one would-be enforce verdict for a request at `route` whose Host
/// resolved to `community`.
pub(crate) fn record(route: &'static str, community: &Resolution, verdict: Result<(), WouldDeny>) {
    record_for(route, label(community), verdict);
}

/// [`record`] for a caller that resolved its community label earlier.
pub(crate) fn record_for(
    route: &'static str,
    community: SharedString,
    verdict: Result<(), WouldDeny>,
) {
    if guard_denied() == Some(true) {
        return;
    }
    // Dropping the would-deny's line writes it now.
    drop(record_pending(route, community, verdict));
}

/// Count `verdict`, returning a would-deny's line: already written when the
/// verdict carries a key, else unwritten until the client proves one.
pub(crate) fn record_pending(
    route: &'static str,
    community: SharedString,
    verdict: Result<(), WouldDeny>,
) -> Option<PendingLine> {
    let (stage, outcome) = match verdict {
        Ok(()) => ("admit", "admit"),
        Err(deny) => (deny.stage.label(), class_label(deny.class)),
    };
    metrics::counter!(
        "buzz_nip_fi_shadow_total",
        "route" => route,
        "stage" => stage,
        "outcome" => outcome,
        "community" => community.clone()
    )
    .increment(1);
    // The counter is the record; only a would-deny earns a line.
    let deny = verdict.err()?;
    let line = PendingLine {
        route,
        stage,
        outcome,
        reason: deny.reason,
        community,
        written: std::sync::atomic::AtomicBool::new(false),
    };
    if let Some(key) = deny.pubkey {
        line.prove(key);
    }
    Some(line)
}

/// A counted would-deny's info line, written exactly once: by the first key
/// proven for it, else as `absent` when finished or dropped.
pub(crate) struct PendingLine {
    route: &'static str,
    stage: &'static str,
    outcome: &'static str,
    reason: &'static str,
    community: SharedString,
    written: std::sync::atomic::AtomicBool,
}

impl PendingLine {
    /// Write the line for `pubkey`, a key the client proved, unless it is
    /// already written.
    pub(crate) fn prove(&self, pubkey: PublicKey) {
        self.write(&pubkey.to_hex());
    }

    /// Write the line as `absent` unless a proof already wrote it.
    pub(crate) fn finish(&self) {
        self.write("absent");
    }

    fn write(&self, pubkey: &str) {
        if self.written.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return;
        }
        tracing::info!(
            route = self.route,
            stage = self.stage,
            outcome = self.outcome,
            reason = self.reason,
            pubkey,
            community = %self.community,
            "nip-fi shadow would-deny"
        );
    }
}

impl Drop for PendingLine {
    fn drop(&mut self) {
        self.finish();
    }
}

/// The request's community, from the router guard's resolution when there is
/// one.
async fn resolution(state: &crate::state::AppState, headers: &HeaderMap) -> Resolution {
    crate::nip_fi_http::resolve_for_admission(state.nip_fi_communities(), headers).await
}

tokio::task_local! {
    /// Set by the router guard in shadow: the line of enforce's guard
    /// would-deny, if any, written when its handler proves a key, else as
    /// `absent` when the request ends.
    static GUARD_DENIED: Option<PendingLine>;
}

/// Count the router guard's would-deny, then run the request inside its
/// verdict.  Enforce never lets a guard-denied request reach a handler, so
/// none of that request's handler-side records are emitted; the guard's line
/// is written when its handler proves a key, else as `absent` when the
/// request ends.
pub(crate) async fn observe_guard<F: std::future::Future>(
    community: SharedString,
    verdict: Result<(), WouldDeny>,
    run: F,
) -> F::Output {
    let line = verdict
        .is_err()
        .then(|| record_pending("http", community, verdict))
        .flatten();
    GUARD_DENIED.scope(line, run).await
}

/// The enclosing router guard's verdict; `None` outside a guarded request.
fn guard_denied() -> Option<bool> {
    GUARD_DENIED.try_with(Option::is_some).ok()
}

/// Attribute the enclosing router guard's would-deny line to the request's
/// verified NIP-98 key.
pub(crate) fn prove_guard(signed: Option<PublicKey>) {
    if let Some(key) = signed {
        let _ = GUARD_DENIED.try_with(|line| line.as_ref().map(|l| l.prove(key)));
    }
}

const fn class_label(class: DenialClass) -> &'static str {
    match class {
        DenialClass::MissingEvidence => "missing",
        DenialClass::EvidenceRejected => "rejected",
        DenialClass::AuthorizationDenied => "denied",
        DenialClass::AuthorizationUnavailable => "unavailable",
    }
}

/// In shadow mode only, record whether `strict` (a pure check) would reject,
/// on its own counter: a pass is one proof, not an admission verdict.
pub(crate) async fn observe_strict_proof<E>(
    state: &crate::state::AppState,
    headers: &HeaderMap,
    route: &'static str,
    strict: impl FnOnce() -> Result<(), E>,
) {
    let nip_fi = &state.config.nip_fi;
    if nip_fi.mode.observes_only() && guard_denied() != Some(true) {
        let outcome = strict().map_or(class_label(DenialClass::EvidenceRejected), |()| "pass");
        let community = label(&resolution(state, headers).await);
        metrics::counter!(
            "buzz_nip_fi_shadow_strict_proof_total",
            "route" => route,
            "outcome" => outcome,
            "community" => community
        )
        .increment(1);
    }
}

/// Bind the request's tenant from its Host header, as every HTTP handler's
/// row zero does. `None` means the Host bound no community; the caller keeps
/// its own legacy rejection, and shadow records enforce's verdict first.
pub(crate) async fn bind_tenant(
    state: &crate::state::AppState,
    headers: &HeaderMap,
) -> Option<buzz_core::TenantContext> {
    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let tenant = crate::tenant::bind_community(state, raw_host).await.ok();
    if tenant.is_none() {
        observe_unbound(state, headers).await;
    }
    tenant
}

/// In shadow mode, record the verdict enforce reaches for a request whose
/// Host bound no tenant: `community` when the Host maps to no configured
/// community; an admit when enforce's guard passed it, since the handler's
/// own rejection is no NIP-FI denial; nothing outside a guarded route.
async fn observe_unbound(state: &crate::state::AppState, headers: &HeaderMap) {
    let nip_fi = &state.config.nip_fi;
    if nip_fi.mode.observes_only() {
        let community = resolution(state, headers).await;
        let verdict = match &community {
            Err(class) => Err(WouldDeny::new(Stage::Community, *class, UNRESOLVED)),
            Ok(_) if guard_denied() == Some(false) => Ok(()),
            Ok(_) => return,
        };
        record("http", &community, verdict);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use axum::http::{header::HOST, HeaderValue};
    use buzz_auth::NipFiMode;
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    use crate::nip_fi_core::test_support::{communities, TEST_COMMUNITY_URI, TEST_HOST};

    /// `(community, stage, outcome)` labels of every shadow counter increment.
    fn shadow_counts(recorder: &DebuggingRecorder) -> Vec<(String, String, String)> {
        counts_of(recorder, "buzz_nip_fi_shadow_total")
    }

    /// `(community, stage, outcome)` of every increment of `metric`; a
    /// missing label reads `-`.
    fn counts_of(recorder: &DebuggingRecorder, metric: &str) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        for (key, _, _, value) in recorder.snapshotter().snapshot().into_vec() {
            let key = key.key();
            if key.name() != metric {
                continue;
            }
            if metric == "buzz_nip_fi_shadow_total" {
                assert_shadow_schema(key);
            }
            let label = |name| {
                key.labels()
                    .find(|l| l.key() == name)
                    .map_or("-", |l| l.value())
                    .to_owned()
            };
            let DebugValue::Counter(n) = value else {
                unreachable!()
            };
            for _ in 0..n {
                out.push((label("community"), label("stage"), label("outcome")));
            }
        }
        out.sort();
        out
    }

    fn with_host(host: Option<&'static str>) -> axum::http::HeaderMap {
        let mut headers = axum::http::HeaderMap::new();
        if let Some(host) = host {
            headers.insert(HOST, HeaderValue::from_static(host));
        }
        headers
    }

    // Mutation: labelling by the raw Host leaks `attacker.example`; labelling
    // an `env` community `bound` drops its per-community series.
    #[tokio::test(flavor = "current_thread")]
    async fn env_label_is_configured_uri_or_unmapped() {
        let map = communities();
        let recorder = DebuggingRecorder::new();
        let _guard = metrics::set_default_local_recorder(&recorder);
        for host in [Some(TEST_HOST), Some("attacker.example"), None] {
            let community = crate::nip_fi_core::Communities::from(&map)
                .resolve(&with_host(host))
                .await;
            super::record("http", &community, Ok(()));
        }
        let labels: Vec<String> = shadow_counts(&recorder)
            .into_iter()
            .map(|(c, _, _)| c)
            .collect();
        assert_eq!(labels, [TEST_COMMUNITY_URI, "unmapped", "unmapped"]);
    }

    struct CountingReplayGuard(AtomicUsize);

    impl buzz_auth::Nip98ReplayGuard for CountingReplayGuard {
        fn try_mark_in_scope<'a>(
            &'a self,
            _scope: &'a str,
            _event_id: &'a nostr::EventId,
            _ttl_secs: u64,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<bool, buzz_auth::AuthError>> + Send + 'a>,
        > {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(true) })
        }
    }

    // A NIP-98 event without a `payload` tag passes lax checks but fails the
    // strict one. Mutations: running the side check outside shadow,
    // claiming the event in the replay guard, or recording either outcome on
    // the admission counter → RED.
    #[tokio::test(flavor = "current_thread")]
    async fn strict_proof_side_check_records_only_in_shadow_and_spares_replay() {
        const STRICT: &str = "buzz_nip_fi_shadow_strict_proof_total";
        use base64::Engine as _;
        let url = "https://relay.example/api/bridge";
        let event = nostr::EventBuilder::new(nostr::Kind::HttpAuth, "")
            .tags([
                nostr::Tag::parse(["u", url]).unwrap(),
                nostr::Tag::parse(["method", "POST"]).unwrap(),
            ])
            .sign_with_keys(&nostr::Keys::generate())
            .unwrap();
        let mut headers = with_host(Some(TEST_HOST));
        let auth = base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_string(&event).unwrap());
        headers.insert(
            axum::http::header::AUTHORIZATION,
            format!("Nostr {auth}").parse().unwrap(),
        );

        let base = crate::state::tests::test_state().await;
        for (mode, expected) in [
            (NipFiMode::Off, 0),
            (NipFiMode::Enforce, 0),
            (NipFiMode::Shadow, 1),
        ] {
            let guard = Arc::new(CountingReplayGuard(AtomicUsize::new(0)));
            let mut state = (*base).clone();
            let mut config = (*state.config).clone();
            config.nip_fi.mode = mode;
            config.nip_fi.communities = communities();
            state.config = Arc::new(config);
            state.nip98_replay = guard.clone();
            let recorder = DebuggingRecorder::new();
            {
                let _guard = metrics::set_default_local_recorder(&recorder);
                super::observe_strict_proof(&state, &headers, "bridge", || {
                    crate::api::bridge::verify_bridge_auth_with_options(
                        &headers,
                        "POST",
                        url,
                        Some(b"{}"),
                        true,
                        true,
                    )
                    .map(drop)
                })
                .await;
            }
            let counts = counts_of(&recorder, STRICT);
            assert_eq!(counts.len(), expected, "{mode:?}");
            assert!(
                counts.iter().all(|(.., outcome)| outcome == "rejected"),
                "{mode:?}"
            );
            assert!(shadow_counts(&recorder).is_empty(), "{mode:?}");
            assert_eq!(guard.0.load(Ordering::SeqCst), 0, "{mode:?}");
            if mode.observes_only() {
                // Both outcomes stay on the proof counter, never an admit
                // that could be summed with whole-request verdicts.
                let recorder = DebuggingRecorder::new();
                {
                    let _guard = metrics::set_default_local_recorder(&recorder);
                    super::observe_strict_proof(&state, &headers, "bridge", || Ok::<_, ()>(()))
                        .await;
                    super::observe_strict_proof(&state, &headers, "bridge", || Err(())).await;
                }
                let label = |outcome: &str| {
                    (
                        TEST_COMMUNITY_URI.to_owned(),
                        "-".to_owned(),
                        outcome.to_owned(),
                    )
                };
                let counts = counts_of(&recorder, STRICT);
                assert_eq!(counts, [label("pass"), label("rejected")]);
                assert!(shadow_counts(&recorder).is_empty());
            }
        }
    }

    #[derive(Clone, Default)]
    struct Captured(Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Run `body` under an info-level subscriber; return its result and the
    /// `nip-fi shadow would-deny` lines it logged.
    pub(crate) fn would_deny_lines<R>(body: impl FnOnce() -> R) -> (R, Vec<String>) {
        let log = WouldDenyLog::capture();
        let result = body();
        (result, log.lines())
    }

    /// An info-level subscriber, default on this thread while held, whose
    /// `nip-fi shadow would-deny` lines can be read at any point.
    pub(crate) struct WouldDenyLog {
        out: Captured,
        _default: tracing::subscriber::DefaultGuard,
        _second: tracing::Dispatch,
        _tracing: std::sync::MutexGuard<'static, ()>,
    }

    impl WouldDenyLog {
        pub(crate) fn capture() -> Self {
            let _tracing = crate::test_support::tracing_dispatch_lock();
            let out = Captured::default();
            let writer = out.clone();
            let subscriber = tracing_subscriber::fmt()
                .with_max_level(tracing::Level::INFO)
                .with_writer(move || writer.clone())
                .with_ansi(false)
                .finish();
            // With one live dispatcher, tracing-core caches a callsite's
            // interest from whichever thread first hits it, so a parallel test
            // without a subscriber can disable this one's would-deny line. A
            // second live dispatcher makes registration consult every one.
            let _second = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
            let _default = tracing::subscriber::set_default(subscriber);
            Self {
                out,
                _default,
                _second,
                _tracing,
            }
        }

        /// The would-deny lines logged so far.
        pub(crate) fn lines(&self) -> Vec<String> {
            let log = String::from_utf8(self.out.0.lock().unwrap().clone()).unwrap();
            log.lines()
                .filter(|l| l.contains("nip-fi shadow would-deny"))
                .map(str::to_owned)
                .collect()
        }
    }

    /// The counted line of a `route` upgrade would-deny for a missing
    /// assertion, as `check_nip_fi_at_upgrade` hands it to the connection.
    pub(crate) fn upgrade_line(route: &'static str) -> Arc<super::PendingLine> {
        let missing = crate::nip_fi_core::AssertionRejection::Transport(
            buzz_auth::DenialClass::MissingEvidence,
        );
        let verdict = Err(super::WouldDeny::assertion(missing));
        Arc::new(super::record_pending(route, super::BOUND.into(), verdict).unwrap())
    }

    /// Fail unless a `buzz_nip_fi_shadow_total` key carries exactly the
    /// bounded label set, with a known `route`.
    pub(crate) fn assert_shadow_schema(key: &metrics::Key) {
        let mut names: Vec<&str> = key.labels().map(|l| l.key()).collect();
        names.sort_unstable();
        assert_eq!(names, ["community", "outcome", "route", "stage"], "{key:?}");
        let route = key.labels().find(|l| l.key() == "route").unwrap().value();
        assert!(["http", "ws", "audio"].contains(&route), "{key:?}");
    }

    /// Record `verdicts`; return the would-deny lines and the shadow counter
    /// labels.
    fn record_logged(
        verdicts: Vec<Result<(), super::WouldDeny>>,
    ) -> (Vec<String>, Vec<(String, String, String)>) {
        let recorder = DebuggingRecorder::new();
        let ((), lines) = would_deny_lines(|| {
            let _guard = metrics::set_default_local_recorder(&recorder);
            for verdict in verdicts {
                super::record_for("http", super::BOUND.into(), verdict);
            }
        });
        (lines, shadow_counts(&recorder))
    }

    // Mutations: logging at debug, dropping `reason` or `pubkey`, logging an
    // admit, or adding either field to the counter's labels → RED.
    #[test]
    fn would_deny_logs_reason_and_pubkey_at_info_off_the_metric() {
        use super::{Stage, WouldDeny};
        use buzz_auth::DenialClass;
        let key = nostr::Keys::generate().public_key();
        let (lines, counts) = record_logged(vec![
            Err(WouldDeny::new(
                Stage::Owner,
                DenialClass::AuthorizationDenied,
                "owner_banned",
            )
            .by(Some(key))),
            Err(WouldDeny::new(
                Stage::Nip98,
                DenialClass::MissingEvidence,
                "missing_proof",
            )),
            Ok(()),
        ]);
        let log = lines.join("\n");
        assert_eq!(lines.len(), 2, "{log}");
        assert!(lines
            .iter()
            .all(|l| l.contains(" INFO ") && l.contains("nip-fi shadow would-deny")));
        assert!(lines[0].contains("reason=\"owner_banned\""), "{log}");
        assert!(
            lines[0].contains(&format!("pubkey=\"{}\"", key.to_hex())),
            "{log}"
        );
        assert!(lines[1].contains("reason=\"missing_proof\""), "{log}");
        assert!(lines[1].contains("pubkey=\"absent\""), "{log}");
        let label =
            |stage: &str, outcome: &str| ("bound".to_owned(), stage.to_owned(), outcome.to_owned());
        assert_eq!(
            counts,
            [
                label("admit", "admit"),
                label("nip98", "missing"),
                label("owner", "denied")
            ]
        );
    }

    // Each verifier error names itself without the wire prefix, and the
    // assertion step never borrows another step's attribution.
    #[test]
    fn assertion_reason_is_the_verifier_code_unprefixed() {
        use crate::nip_fi_core::AssertionRejection;
        use buzz_auth::DenialClass;
        let deny = super::WouldDeny::assertion(AssertionRejection::Transport(
            DenialClass::MissingEvidence,
        ));
        assert_eq!(deny.reason, "missing_assertion");
        assert_eq!(deny.pubkey, None);
        let deny = super::WouldDeny::assertion(AssertionRejection::VerifierUnavailable);
        assert_eq!(
            (deny.reason, deny.class),
            (
                "verifier_unavailable",
                DenialClass::AuthorizationUnavailable
            )
        );
        let deny = super::WouldDeny::assertion(AssertionRejection::Verifier(
            buzz_auth::VerifierError::Expired,
        ));
        assert_eq!(deny.reason, "expired");
    }

    /// Mode semantics live in `NipFiMode`'s predicates. Naming a variant in
    /// production code anywhere else (`==`, `matches!`, `if let`, or-patterns,
    /// match arms) is how shadow silently inherits Off or Enforce behavior.
    #[test]
    fn nip_fi_mode_is_inspected_only_through_predicates() {
        const ALLOWED: [&str; 2] = [
            "buzz-auth/src/nip_fi/startup/mod.rs",
            "buzz-relay/src/nip_fi_config.rs",
        ];
        let crates = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
        let mut offenders = Vec::new();
        let mut dirs: Vec<_> = std::fs::read_dir(crates)
            .expect("read crates dir")
            .map(|e| e.expect("crates entry").path().join("src"))
            .filter(|src| src.is_dir())
            .collect();
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).expect("read source dir") {
                let path = entry.expect("source entry").path();
                if path.is_dir() {
                    dirs.push(path);
                    continue;
                }
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                if !name.ends_with(".rs") || name == "tests.rs" || name.ends_with("_tests.rs") {
                    continue;
                }
                let shown = path
                    .strip_prefix(crates)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                if ALLOWED.contains(&shown.as_str()) {
                    continue;
                }
                let text = std::fs::read_to_string(&path).expect("read source file");
                for (i, line) in production_lines(&text).enumerate() {
                    let code = line.split("//").next().unwrap_or_default();
                    if ["Off", "Shadow", "Enforce", "DenyProtected"]
                        .iter()
                        .any(|v| code.contains(&format!("NipFiMode::{v}")))
                    {
                        offenders.push(format!("{shown}:{}", i + 1));
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "raw NipFiMode variants: {offenders:?}"
        );
    }

    /// Lines before the first inline `#[cfg(test)] mod x {`; an out-of-line
    /// `#[cfg(test)] mod x;` declaration does not end production code.
    fn production_lines(text: &str) -> impl Iterator<Item = &str> {
        let mut lines = text.lines().peekable();
        std::iter::from_fn(move || {
            let line = lines.next()?;
            let inline_test_mod = line.trim() == "#[cfg(test)]"
                && lines.peek().is_some_and(|n| {
                    let n = n.trim_start().trim_start_matches("pub(crate) ");
                    n.starts_with("mod ") && n.trim_end().ends_with('{')
                });
            (!inline_test_mod).then_some(line) // ends the iterator
        })
    }
}
