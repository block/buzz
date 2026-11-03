//! Request-scoped community policy. Each request reads the writer once; no
//! cross-request policy cache can publish a stale optional generation over an
//! observed requirement. Existing requests/sessions are drained by BUZZ-512.

use axum::{
    body::Body,
    http::{HeaderMap, Response, StatusCode},
    response::IntoResponse,
};
use buzz_auth::{DenialClass, NipFiMode};
use buzz_db::community_auth::CommunityAuthPolicy;
use futures_util::future::{BoxFuture, FutureExt};

/// Host policy source, shared by admission and discovery.
pub(crate) trait PolicySource: Send + Sync {
    fn lookup<'a>(
        &'a self,
        host: &'a str,
    ) -> BoxFuture<'a, Result<Option<CommunityAuthPolicy>, ()>>;
}
impl PolicySource for buzz_db::Db {
    fn lookup<'a>(
        &'a self,
        host: &'a str,
    ) -> BoxFuture<'a, Result<Option<CommunityAuthPolicy>, ()>> {
        async move {
            self.community_auth_policy_for_host(host)
                .await
                .map_err(|_| ())
        }
        .boxed()
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Policy {
    pub mode: NipFiMode,
    pub required: bool,
    pub stored_required: bool,
}

tokio::task_local! { static REQUEST_POLICY: Policy; }

pub(crate) async fn with_policy<F: std::future::Future>(policy: Policy, next: F) -> F::Output {
    REQUEST_POLICY.scope(policy, next).await
}

/// Repair mode precedes all policy and evidence failures.
pub(crate) fn effective(global: NipFiMode, required: bool) -> NipFiMode {
    global.with_requirement(required)
}

pub(crate) async fn resolve(
    state: &crate::state::AppState,
    headers: &HeaderMap,
) -> Result<Policy, Response<Body>> {
    if let Ok(policy) = REQUEST_POLICY.try_with(|policy| *policy) {
        return Ok(policy);
    }
    let global = state.config.nip_fi.mode;
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let host = buzz_core::tenant::normalize_host(host);
    if host.is_empty() {
        return Err(not_found());
    }
    let start = std::time::Instant::now();
    let read = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        state.nip_fi_policy.lookup(&host),
    )
    .await;
    metrics::histogram!("buzz_nip_fi_policy_read_seconds").record(start.elapsed().as_secs_f64());
    metrics::counter!("buzz_nip_fi_policy_reads_total").increment(1);
    let policy = match read {
        Ok(Ok(Some(policy))) => policy,
        Ok(Ok(None)) => return Err(not_found()),
        _ => {
            return Err(crate::nip_fi_core::http_denial(
                DenialClass::AuthorizationUnavailable,
            ))
        }
    };
    tracing::trace!(
        generation = policy.policy_generation,
        "resolved community auth policy"
    );
    Ok(Policy {
        mode: effective(global, policy.requires_nip_fi),
        required: policy.requires_nip_fi || global.enforces(),
        stored_required: policy.requires_nip_fi,
    })
}

/// Admission callers share the router's decision. Direct callers fail closed
/// when policy is unavailable, even if the global rollout mode is Off/Shadow.
pub(crate) async fn mode(state: &crate::state::AppState, headers: &HeaderMap) -> NipFiMode {
    if state.config.nip_fi.mode.denies_unconditionally() {
        return NipFiMode::unavailable();
    }
    match resolve(state, headers).await {
        Ok(p)
            if p.stored_required
                && (state.nip_fi_verifier.is_none()
                    || state.nip_fi_deny_map.is_none()
                    || state.config.nip_fi.max_connection_lifetime().is_none()) =>
        {
            NipFiMode::unavailable()
        }
        Ok(p) => p.mode,
        Err(_) => NipFiMode::unavailable(),
    }
}

fn not_found() -> Response<Body> {
    (
        StatusCode::NOT_FOUND,
        "relay: no community is configured for this host",
    )
        .into_response()
}

#[cfg(test)]
pub(crate) struct OptionalPolicy;
#[cfg(test)]
impl PolicySource for OptionalPolicy {
    fn lookup<'a>(
        &'a self,
        _host: &'a str,
    ) -> BoxFuture<'a, Result<Option<CommunityAuthPolicy>, ()>> {
        async {
            Ok(Some(CommunityAuthPolicy {
                requires_nip_fi: false,
                policy_generation: 0,
            }))
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod postgres_tests;
