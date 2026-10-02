//! Shadow-mode recording of what enforce would have decided. The verdict
//! never reaches admission, a connection, or a response. Labels are bounded:
//! the community is its configured URI or `unmapped`, never the raw `Host`,
//! and no token, claim, key, or issuer is logged. [FI-TRACE-PRIVACY-NONPUBLIC]

use axum::http::HeaderMap;
use buzz_auth::DenialClass;

use crate::nip_fi_config::NipFiCommunities;

/// Which enforce step would have denied, and with which class.
pub(crate) type WouldDeny = (&'static str, DenialClass);

/// Record one would-be enforce verdict for a request at `route`.
pub(crate) fn record(
    route: &'static str,
    headers: &HeaderMap,
    communities: &NipFiCommunities,
    verdict: Result<(), WouldDeny>,
) {
    record_for(route, community_label(headers, communities), verdict);
}

/// [`record`] for a caller that resolved its community label earlier.
pub(crate) fn record_for(route: &'static str, community: String, verdict: Result<(), WouldDeny>) {
    let (stage, outcome) = match verdict {
        Ok(()) => ("admit", "admit"),
        Err((stage, class)) => (stage, class_label(class)),
    };
    emit(route, community, stage, outcome);
}

/// The bounded community label for `headers`: its configured URI or `unmapped`.
pub(crate) fn community_label(headers: &HeaderMap, communities: &NipFiCommunities) -> String {
    crate::nip_fi_core::resolve_community(headers, communities)
        .map_or("unmapped", |binding| binding.expected_aud())
        .to_owned()
}

tokio::task_local! {
    /// Set by the router guard in shadow: whether enforce's guard would deny.
    static GUARD_DENIED: bool;
}

/// Record the router guard's would-deny, then run the request inside its
/// verdict.  Enforce never lets a guard-denied request reach a handler, so
/// none of that request's handler-side records are emitted.
pub(crate) async fn observe_guard<F: std::future::Future>(
    state: &crate::state::AppState,
    headers: &HeaderMap,
    verdict: Result<(), WouldDeny>,
    run: F,
) -> F::Output {
    let denied = verdict.is_err();
    if denied {
        record("http", headers, &state.config.nip_fi.communities, verdict);
    }
    GUARD_DENIED.scope(denied, run).await
}

/// The enclosing router guard's verdict; `None` outside a guarded request.
fn guard_denied() -> Option<bool> {
    GUARD_DENIED.try_with(|denied| *denied).ok()
}

fn emit(route: &'static str, community: String, stage: &'static str, outcome: &'static str) {
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
    tracing::info!(route, stage, outcome, community, "nip-fi shadow verdict");
}

const fn class_label(class: DenialClass) -> &'static str {
    match class {
        DenialClass::MissingEvidence => "missing",
        DenialClass::EvidenceRejected => "rejected",
        DenialClass::AuthorizationDenied => "denied",
        DenialClass::AuthorizationUnavailable => "unavailable",
    }
}

/// In shadow mode only, record whether `strict` (a pure check) would reject.
/// Its stage is always `strict_proof`: a pass is one proof, not an admit.
pub(crate) fn observe_strict_proof<E>(
    state: &crate::state::AppState,
    headers: &HeaderMap,
    route: &'static str,
    strict: impl FnOnce() -> Result<(), E>,
) {
    let nip_fi = &state.config.nip_fi;
    if nip_fi.mode.observes_only() {
        let outcome = strict().map_or(class_label(DenialClass::EvidenceRejected), |()| "pass");
        let community = community_label(headers, &nip_fi.communities);
        emit(route, community, "strict_proof", outcome);
    }
}

/// In shadow mode, record the verdict enforce reaches for a request whose
/// Host bound no tenant: `community` when the Host maps to no configured
/// community; an admit when enforce's guard passed it, since the handler's
/// own rejection is no NIP-FI denial; nothing outside a guarded route.
pub(crate) fn observe_unbound(state: &crate::state::AppState, headers: &HeaderMap) {
    let nip_fi = &state.config.nip_fi;
    if nip_fi.mode.observes_only() {
        let verdict = match crate::nip_fi_core::resolve_community(headers, &nip_fi.communities) {
            Err(class) => Err(("community", class)),
            Ok(_) if guard_denied() == Some(false) => Ok(()),
            Ok(_) => return,
        };
        record("http", headers, &nip_fi.communities, verdict);
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
        let mut out = Vec::new();
        for (key, _, _, value) in recorder.snapshotter().snapshot().into_vec() {
            let key = key.key();
            if key.name() != "buzz_nip_fi_shadow_total" {
                continue;
            }
            let label = |name| {
                key.labels()
                    .find(|l| l.key() == name)
                    .unwrap()
                    .value()
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

    // Mutation: labelling by the raw Host leaks `attacker.example`.
    #[test]
    fn community_label_is_configured_uri_or_unmapped() {
        let recorder = DebuggingRecorder::new();
        metrics::with_local_recorder(&recorder, || {
            for host in [Some(TEST_HOST), Some("attacker.example"), None] {
                super::record("http", &with_host(host), &communities(), Ok(()));
            }
        });
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
    // claiming the event in the replay guard, or recording a pass as
    // `stage=admit` → RED.
    #[tokio::test(flavor = "current_thread")]
    async fn strict_proof_side_check_records_only_in_shadow_and_spares_replay() {
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
            metrics::with_local_recorder(&recorder, || {
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
                });
            });
            let counts = shadow_counts(&recorder);
            assert_eq!(counts.len(), expected, "{mode:?}");
            assert!(
                counts
                    .iter()
                    .all(|(_, stage, outcome)| stage == "strict_proof" && outcome == "rejected"),
                "{mode:?}"
            );
            assert_eq!(guard.0.load(Ordering::SeqCst), 0, "{mode:?}");
            if mode.observes_only() {
                // Both outcomes stay `strict_proof`, never an admit that
                // could be summed with whole-request verdicts.
                let recorder = DebuggingRecorder::new();
                metrics::with_local_recorder(&recorder, || {
                    super::observe_strict_proof(&state, &headers, "bridge", || Ok::<_, ()>(()));
                    super::observe_strict_proof(&state, &headers, "bridge", || Err(()));
                });
                let label = |outcome: &str| {
                    (
                        "https://relay.example".to_owned(),
                        "strict_proof".to_owned(),
                        outcome.to_owned(),
                    )
                };
                assert_eq!(shadow_counts(&recorder), [label("pass"), label("rejected")]);
            }
        }
    }

    /// Mode semantics live in `NipFiMode`'s predicates. A raw `matches!` on a
    /// mode elsewhere is how shadow silently inherits Off or Enforce
    /// behavior, so production code must not contain one.
    #[test]
    fn nip_fi_mode_is_inspected_only_through_predicates() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
        let mut offenders = Vec::new();
        let mut dirs = vec![std::path::PathBuf::from(root)];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if !path.ends_with("target") {
                        dirs.push(path);
                    }
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs")
                    || path.file_name().is_some_and(|n| n == "tests.rs")
                    || path.ends_with("nip_fi/startup/mod.rs")
                {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                let production = text.split("\n#[cfg(test)]\nmod ").next().unwrap();
                let lines: Vec<&str> = production.lines().collect();
                for (i, line) in lines.iter().enumerate() {
                    if line.trim_start().starts_with("//") {
                        continue;
                    }
                    if line.contains("matches!") && line.contains("NipFiMode::")
                        || line.trim_start().starts_with("NipFiMode::") && line.contains(" | ")
                        || line.contains("NipFiMode::")
                            && line.contains("=>")
                            && wildcard_follows(&lines[i + 1..])
                    {
                        offenders.push(format!("{}:{}", path.display(), i + 1));
                    }
                }
            }
        }
        assert!(offenders.is_empty(), "raw NipFiMode matches: {offenders:?}");
    }

    /// A `_` arm shortly after a mode arm lets new modes fall through silently.
    fn wildcard_follows(rest: &[&str]) -> bool {
        rest.iter()
            .take(4)
            .any(|l| l.trim_start().starts_with("_ =>"))
    }
}
