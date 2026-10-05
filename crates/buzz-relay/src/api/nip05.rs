//! NIP-05 identity verification endpoint.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::{HeaderMap, HeaderValue},
    response::{IntoResponse, Json, Response},
};
use hex;
use serde::Deserialize;

use crate::state::AppState;

/// Query parameters for the NIP-05 identity verification endpoint.
#[derive(Deserialize)]
pub struct Nip05Query {
    /// The local part of the NIP-05 identifier to look up (e.g. `alice` from `alice@relay.example`).
    pub name: Option<String>,
}

/// `GET /.well-known/nostr.json` — NIP-05 identity verification.
/// No authentication required — public discovery endpoint.
pub async fn nostr_nip05(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<Nip05Query>,
) -> Response {
    // Row zero: bind this public request to its community from the request host
    // before the tenant-scoped lookup, identical to the bridge/WS doors. An
    // unmapped host falls through to the empty `{names,relays}` response — never
    // a default tenant, never echoing which communities exist on this deployment.
    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let json = match (
        params.name,
        crate::tenant::bind_community(&*state, raw_host).await,
    ) {
        (Some(n), Ok(tenant)) => {
            let name = n.to_lowercase();
            // NIP-05 identity is host-scoped in a multi-tenant relay: the
            // community is already bound from Host, and the handle domain must
            // match that same tenant host (not process-global config.relay_url).
            let domain = extract_domain(tenant.host());
            match state
                .db
                .get_user_by_nip05(tenant.community(), &name, &domain)
                .await
            {
                Ok(Some(user)) => {
                    let hex_pubkey = hex::encode(&user.pubkey);
                    let relay_url =
                        relay_url_for_tenant_host(&state.config.relay_url, tenant.host());
                    serde_json::json!({
                        "names": { (name): hex_pubkey.clone() },
                        "relays": { (hex_pubkey): [relay_url] }
                    })
                }
                _ => serde_json::json!({ "names": {}, "relays": {} }),
            }
        }
        _ => serde_json::json!({ "names": {}, "relays": {} }),
    };

    let mut response = Json(json).into_response();
    response.headers_mut().insert(
        axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    response
}

// The canonicalization rule lives in `buzz_core::nip05` so the kind-0 `users`
// projection (`buzz_db::ProfileProjection`, written on relay ingest) and the
// `buzz-admin` profile backfill apply the same definition the lookup endpoint
// reads back through `extract_domain`.
#[cfg(test)]
use buzz_core::nip05::canonicalize_nip05;
pub(crate) use buzz_core::nip05::extract_domain;

/// Build the relay URL advertised in the NIP-05 `relays` map for the bound
/// tenant. Preserve the deployment scheme from config, but never the configured
/// host: in a host-per-community deployment the request tenant host is the
/// relay identity clients must follow back.
pub(crate) fn relay_url_for_tenant_host(config_relay_url: &str, tenant_host: &str) -> String {
    let scheme = if config_relay_url.trim_start().starts_with("wss://") {
        "wss"
    } else {
        "ws"
    };
    format!("{scheme}://{tenant_host}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_nip05_accepts_bound_tenant_host_not_config_url() {
        assert_eq!(
            canonicalize_nip05("Alice@tenant-b.example", "tenant-b.example").unwrap(),
            "alice@tenant-b.example"
        );
        assert!(canonicalize_nip05("alice@config.example", "tenant-b.example").is_err());
    }

    #[test]
    fn relay_url_for_tenant_host_uses_config_scheme_but_tenant_host() {
        assert_eq!(
            relay_url_for_tenant_host("wss://config.example", "tenant-b.example"),
            "wss://tenant-b.example"
        );
        assert_eq!(
            relay_url_for_tenant_host("ws://config.example", "localhost:3100"),
            "ws://localhost:3100"
        );
    }
}
