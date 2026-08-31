//! Selected-owner private host inventory, authorization and execution history.
//! These POST bodies must never follow redirects, even when sensitive HTTP
//! headers would be stripped: the filters themselves disclose private metadata.
use super::*;

/// Per-request deadline includes response-body consumption, not rate-limit wait.
/// Private host reads/writes deliberately use a shorter budget than WS history.
pub(crate) const PRIVATE_HOST_REQUEST_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(15);

/// Read private host records with the current unlocked owner's authority, using
/// the same no-redirect client and deadline as host execution publication.
pub(crate) async fn query_private_host_at_with_keys(
    state: &AppState,
    api_base_url: &str,
    filters: &[serde_json::Value],
    keys: &Keys,
    auth_tag: Option<&str>,
) -> Result<Vec<nostr::Event>, String> {
    crate::relay_admission::wait_for_rate_limit().await;
    assert_expected_signer(
        Some(&keys.public_key().to_hex()),
        &state.signing_keys()?.public_key().to_hex(),
    )?;
    let url = format!("{}/query", api_base_url.trim_end_matches('/'));
    let body =
        serde_json::to_vec(filters).map_err(|e| format!("filter serialization failed: {e}"))?;
    let auth = build_nip98_auth_header_for_keys(keys, &Method::POST, &url, &body)?;
    send_query_request(
        &state.media_fetch_client,
        &url,
        &auth,
        auth_tag,
        body,
        PRIVATE_HOST_REQUEST_TIMEOUT,
    )
    .await
}

#[cfg(test)]
mod tests;
