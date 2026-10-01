//! Shadow-mode recording: what enforce mode would have decided.
//!
//! In [`NipFiMode::Shadow`](buzz_auth::NipFiMode::Shadow) every ingress admits
//! exactly as `Off` and additionally evaluates the evidence the way `Enforce`
//! would. The verdict is reported here and nowhere else — it never reaches an
//! admission decision, a connection, or a response.
//!
//! Recorded labels are bounded and non-identifying: the ingress, the enforce
//! step that would have denied, the denial class, and the community's
//! configured canonical URI (or `unmapped`). No token bytes, claims, keys,
//! issuers, or raw `Host` values are logged. [FI-TRACE-PRIVACY-NONPUBLIC]

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
    let community = crate::nip_fi_core::resolve_community(headers, communities)
        .map_or("unmapped", |binding| binding.expected_aud())
        .to_owned();
    let (stage, outcome) = match verdict {
        Ok(()) => ("admit", "admit"),
        Err((stage, class)) => (stage, class_label(class)),
    };
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

/// In shadow mode, run `strict` — the enforce-mode proof check that is pure
/// and consumes nothing — and record whether it would have rejected.
/// No-op in every other mode.
pub(crate) fn observe_strict_proof<E>(
    state: &crate::state::AppState,
    headers: &HeaderMap,
    route: &'static str,
    strict: impl FnOnce() -> Result<(), E>,
) {
    let nip_fi = &state.config.nip_fi;
    if nip_fi.mode.observes_only() {
        let verdict = strict().map_err(|_| ("strict_proof", DenialClass::EvidenceRejected));
        record(route, headers, &nip_fi.communities, verdict);
    }
}

#[cfg(test)]
mod tests {
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
                let production = text.split("#[cfg(test)]\nmod tests").next().unwrap();
                for (i, line) in production.lines().enumerate() {
                    if line.trim_start().starts_with("//") {
                        continue;
                    }
                    if line.contains("matches!") && line.contains("NipFiMode::")
                        || line.trim_start().starts_with("NipFiMode::") && line.contains(" | ")
                    {
                        offenders.push(format!("{}:{}", path.display(), i + 1));
                    }
                }
            }
        }
        assert!(offenders.is_empty(), "raw NipFiMode matches: {offenders:?}");
    }
}
