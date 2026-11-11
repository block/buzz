//! Host policy uses short primary-read leases, shared with tenant routing.
//! Acceptance excludes new leases and drains old ones before commit. Request
//! scopes retain one snapshot; existing work is drained separately by BUZZ-512.

use axum::{
    body::Body,
    http::{HeaderMap, Response, StatusCode},
    response::IntoResponse,
};
use buzz_auth::{DenialClass, NipFiMode};
use buzz_db::community_auth::CommunityAuthPolicy;
use futures_util::future::{BoxFuture, FutureExt};

pub(crate) const POLICY_READ_BUDGET: std::time::Duration = std::time::Duration::from_millis(500);

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
            self.community_auth_policy_lease_for_host(host)
                .await
                .map_err(|_| ())
        }
        .boxed()
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Policy {
    pub community_id: uuid::Uuid,
    pub mode: NipFiMode,
    pub required: bool,
    pub stored_required: bool,

    // False only for the guard's legacy rejection scope after an authoritative
    // host miss. Such a scope must never yield an admission proof.
    pub mapped: bool,
}

impl Policy {
    /// Bind tenant-scoped data to the row that supplied admission policy.
    pub(crate) fn tenant(self, raw_host: &str) -> Option<buzz_core::TenantContext> {
        let host = buzz_core::tenant::normalize_host(raw_host);
        (self.mapped && !host.is_empty()).then(|| {
            buzz_core::TenantContext::resolved(
                buzz_core::CommunityId::from_uuid(self.community_id),
                host,
            )
        })
    }
}

tokio::task_local! { static REQUEST_POLICY: Policy; }

/// None outside a guarded request; Some(None) is an authoritative host miss.
/// A guarded request must never fall back to another host-cache snapshot.
pub(crate) fn request_tenant(raw_host: &str) -> Option<Option<buzz_core::TenantContext>> {
    REQUEST_POLICY
        .try_with(|policy| policy.tenant(raw_host))
        .ok()
}

/// Effective mode already resolved for this request, without another lookup.
pub(crate) fn request_mode(global: NipFiMode) -> NipFiMode {
    REQUEST_POLICY
        .try_with(|policy| policy.mode)
        .unwrap_or(global)
}

pub(crate) async fn with_policy<F: std::future::Future>(policy: Policy, next: F) -> F::Output {
    REQUEST_POLICY.scope(policy, next).await
}

/// Repair mode precedes all policy and evidence failures.
pub(crate) fn effective(global: NipFiMode, required: bool) -> NipFiMode {
    global.with_requirement(required)
}

/// One bounded policy fill. Its serving row also supplies ordinary routing.
/// Misses, errors, expired reads and invalidated in-flight results are not cached.
pub(crate) async fn cached_lookup(
    state: &crate::state::AppState,
    host: &str,
) -> Result<Option<CommunityAuthPolicy>, ()> {
    tokio::time::timeout(POLICY_READ_BUDGET, async {
        if let Some(policy) = state.tenant_host_cache.policy(host) {
            metrics::counter!("buzz_nip_fi_policy_cache_hits_total").increment(1);
            return Ok(Some(policy));
        }
        let _turn = state.tenant_host_reads.turn(host.to_owned()).await;
        if let Some(policy) = state.tenant_host_cache.policy(host) {
            metrics::counter!("buzz_nip_fi_policy_cache_hits_total").increment(1);
            return Ok(Some(policy));
        }
        // Misses and errors are not cached. A waiter with no shared result
        // retries rather than inventing a 503 for an unmapped host.
        metrics::counter!("buzz_nip_fi_policy_cache_misses_total").increment(1);
        let epoch = state.tenant_host_cache.epoch();
        // Anchor before checkout, not at query completion or cache insertion.
        let read_at = std::time::Instant::now();
        metrics::counter!("buzz_nip_fi_policy_reads_total").increment(1);
        let result = state.nip_fi_policy.lookup(host).await;
        metrics::histogram!("buzz_nip_fi_policy_read_seconds")
            .record(read_at.elapsed().as_secs_f64());
        match result? {
            // Invalidation prevents publication, not the use of a snapshot
            // already read before commit. Such a request is in flight, just
            // like an uncached primary reader; activation fences its work.
            Some(policy) => Ok(Some(
                state
                    .tenant_host_cache
                    .publish(host.to_owned(), policy, read_at, epoch)
                    .unwrap_or(policy),
            )),
            None => Ok(None),
        }
    })
    .await
    .map_err(|_| ())?
}

#[allow(clippy::result_large_err)] // Axum responses are the ingress error contract.
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
    if host.is_empty() || host.chars().count() > buzz_core::tenant::MAX_HOST_LEN {
        return Err(not_found());
    }
    let policy = match cached_lookup(state, &host).await {
        Ok(Some(policy)) => policy,
        Ok(None) => return Err(not_found()),
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
        community_id: policy.community_id,
        mode: effective(global, policy.requires_nip_fi),
        required: !global.is_off() && (policy.requires_nip_fi || global.enforces()),

        stored_required: policy.requires_nip_fi,
        mapped: true,
    })
}

/// Preserve route-specific rejection ordering for an authoritative host miss.
/// This is not admission: the full gate below still requires a mapped policy.
pub(crate) async fn mode(state: &crate::state::AppState, headers: &HeaderMap) -> NipFiMode {
    if state.config.nip_fi.mode.is_off() {
        return state.config.nip_fi.mode;
    }
    if state.config.nip_fi.mode.denies_unconditionally() {
        return NipFiMode::unavailable();
    }
    match resolve(state, headers).await {
        Ok(policy) => configured_mode(state, policy),
        Err(response) if response.status() == StatusCode::NOT_FOUND => state.config.nip_fi.mode,
        Err(_) => NipFiMode::unavailable(),
    }
}

/// Row-zero binding and the policy belonging to that exact primary row.
/// Early rejection ordering may use `mode`, but cannot authorize an upgrade.
#[allow(clippy::result_large_err)]
pub(crate) async fn bind_for_upgrade(
    state: &crate::state::AppState,
    raw_host: &str,
) -> Result<(buzz_core::tenant::TenantContext, NipFiMode), Response<Body>> {
    if state.config.nip_fi.mode.is_off() {
        return crate::tenant::bind_community(state, raw_host)
            .await
            .map(|tenant| (tenant, state.config.nip_fi.mode))
            .map_err(|_| not_found());
    }
    let host = buzz_core::tenant::normalize_host(raw_host);
    if host.is_empty() || host.chars().count() > buzz_core::tenant::MAX_HOST_LEN {
        return Err(not_found());
    }
    let binding = tokio::time::timeout(
        POLICY_READ_BUDGET,
        state.db.community_auth_binding_for_host(&host),
    )
    .await;
    let (row, policy) = match binding {
        Ok(Ok(Some(binding))) => binding,
        // Row-zero binding preserves its generic refusal on a missing row or
        // failed lookup. Neither outcome can yield a tenant or an upgrade.
        _ => return Err(not_found()),
    };
    let policy = Policy {
        community_id: policy.community_id,
        mode: effective(state.config.nip_fi.mode, policy.requires_nip_fi),
        required: !state.config.nip_fi.mode.is_off()
            && (policy.requires_nip_fi || state.config.nip_fi.mode.enforces()),

        stored_required: policy.requires_nip_fi,
        mapped: true,
    };
    Ok((
        buzz_core::tenant::TenantContext::resolved(row.id, host),
        configured_mode(state, policy),
    ))
}

/// A shared admission proof is impossible without a serving host policy,
/// even when a route preserves legacy unauthenticated error precedence.
#[allow(clippy::result_large_err)] // Axum responses are the ingress error contract.
pub(crate) async fn admission_mode(
    state: &crate::state::AppState,
    headers: &HeaderMap,
) -> Result<NipFiMode, Response<Body>> {
    if state.config.nip_fi.mode.is_off() {
        return Ok(state.config.nip_fi.mode);
    }
    if state.config.nip_fi.mode.denies_unconditionally() {
        return Ok(NipFiMode::unavailable());
    }
    let policy = resolve(state, headers)
        .await
        .map_err(|_| crate::nip_fi_core::http_denial(DenialClass::AuthorizationUnavailable))?;
    if !policy.mapped {
        return Err(crate::nip_fi_core::http_denial(
            DenialClass::AuthorizationUnavailable,
        ));
    }
    Ok(configured_mode(state, policy))
}

pub(crate) fn required_dependencies_ready(state: &crate::state::AppState) -> bool {
    state.nip_fi_verifier.is_some()
        && state.nip_fi_deny_map.is_some()
        && state.config.nip_fi.max_connection_lifetime().is_some()
}

fn configured_mode(state: &crate::state::AppState, policy: Policy) -> NipFiMode {
    if !state.config.nip_fi.mode.is_off()
        && policy.stored_required
        && !required_dependencies_ready(state)
    {
        NipFiMode::unavailable()
    } else {
        policy.mode
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
                community_id: uuid::Uuid::nil(),
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
