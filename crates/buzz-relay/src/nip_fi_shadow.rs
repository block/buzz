//! Shadow-mode recording of what enforce would have decided. The verdict
//! never reaches admission, a connection, or a response. Shadow resolves the
//! community exactly as enforce does, database lookup and freshness included,
//! and once per HTTP request. Labels are bounded and never the raw `Host`: the
//! community label is the configured URI for the `env` source (a set bounded
//! by config), `bound` for the `database` source, and `unmapped` when the
//! community did not resolve. No token, claim, or issuer is logged. The one
//! key logged is the NIP-98 proof key, in the debug line of the key-pairing step shadow shares with
//! enforce. [FI-TRACE-PRIVACY-NONPUBLIC]
//!
//! # Metrics
//!
//! | Series | Labels | One increment per |
//! |---|---|---|
//! | `buzz_nip_fi_shadow_total` | `route` (`http`, `ws`, `audio`), `stage`, `outcome`, `community` | NIP-FI decision enforce would make: one per HTTP request that reaches admission, one per WebSocket upgrade enforce would refuse, and one per observed session: at the AUTH where enforce would admit or deny it, or `deadline` when its deadline passes first. A request or upgrade whose community binding goes stale before enforce's final re-check also records `deadline` |
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

use crate::nip_fi_http::Resolution;

/// Which enforce step would have denied, and with which class.
pub(crate) type WouldDeny = (Stage, DenialClass);

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
        }
    }
}

/// The `community` label of a `database`-source community that resolved.
pub(crate) const BOUND: &str = "bound";
/// The `community` label of a verdict whose community did not resolve.
const UNMAPPED: &str = "unmapped";

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
    let (stage, outcome) = match verdict {
        Ok(()) => ("admit", "admit"),
        Err((stage, class)) => (stage.label(), class_label(class)),
    };
    emit(route, community, stage, outcome);
}

/// The request's community, from the router guard's resolution when there is
/// one.
async fn resolution(state: &crate::state::AppState, headers: &HeaderMap) -> Resolution {
    crate::nip_fi_http::resolve_for_admission(state.nip_fi_communities(), headers).await
}

tokio::task_local! {
    /// Set by the router guard in shadow: whether enforce's guard would deny.
    static GUARD_DENIED: bool;
}

/// Record the router guard's would-deny, then run the request inside its
/// verdict.  Enforce never lets a guard-denied request reach a handler, so
/// none of that request's handler-side records are emitted.
pub(crate) async fn observe_guard<F: std::future::Future>(
    community: SharedString,
    verdict: Result<(), WouldDeny>,
    run: F,
) -> F::Output {
    let denied = verdict.is_err();
    if denied {
        record_for("http", community, verdict);
    }
    GUARD_DENIED.scope(denied, run).await
}

/// The enclosing router guard's verdict; `None` outside a guarded request.
fn guard_denied() -> Option<bool> {
    GUARD_DENIED.try_with(|denied| *denied).ok()
}

fn emit(route: &'static str, community: SharedString, stage: &'static str, outcome: &'static str) {
    if guard_denied() == Some(true) {
        return;
    }
    metrics::counter!(
        "buzz_nip_fi_shadow_total",
        "route" => route,
        "stage" => stage,
        "outcome" => outcome,
        "community" => community.clone()
    )
    .increment(1);
    // The counter is the record; only a would-deny earns a (debug) line.
    if stage != "admit" {
        tracing::debug!(route, stage, outcome, %community, "nip-fi shadow would-deny");
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
    let tenant = crate::tenant::bind_community(&state.db, raw_host)
        .await
        .ok();
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
            Err(class) => Err((Stage::Community, *class)),
            Ok(_) if guard_denied() == Some(false) => Ok(()),
            Ok(_) => return,
        };
        record("http", &community, verdict);
    }
}

#[cfg(test)]
mod tests {
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
                        &crate::access::AccessSlot::default(),
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
