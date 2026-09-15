//! NIP-FI native client: enterprise admission, still local signing.
//!
//! Build configuration is deliberately explicit and disabled by default. The
//! adapter exchange is an assumed API, not a claim that kgoose implements it.
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::STANDARD, Engine};
#[cfg(test)]
use buzz_ws_client_pkg::federated_identity::Assertion;
use buzz_ws_client_pkg::federated_identity::{IdentitySession, IDENTITY_HEADER};
use nostr::PublicKey;
use tauri::Manager;
use tauri::State;

use crate::app_state::AppState;

pub(crate) fn now() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .map_err(|_| "system clock unavailable".into())
}

/// Invalid corporate build config is retained as an error, never an OSS fallback.
pub(crate) fn configured_session() -> Result<Arc<IdentitySession>, String> {
    let origins = option_env!("BUZZ_BUILD_NIP_FI_ORIGINS").unwrap_or("");
    let origins: Vec<&str> = origins
        .split(',')
        .filter(|value| !value.is_empty())
        .collect();
    IdentitySession::new(&origins)
        .map(Arc::new)
        .map_err(|error| error.to_string())
}

pub(crate) fn session(state: &AppState) -> Result<&Arc<IdentitySession>, String> {
    state.federated_identity.as_ref().map_err(Clone::clone)
}

/// Pair with the actual HTTP/Blossom proof, including explicit agent-key paths.
/// Do not silently substitute the currently selected human's key.
pub(crate) fn proof_key(auth: &str) -> Result<PublicKey, String> {
    let encoded = auth
        .strip_prefix("Nostr ")
        .ok_or("invalid local possession proof")?;
    let bytes = STANDARD
        .decode(encoded)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(encoded))
        .map_err(|_| "invalid local possession proof")?;
    let event: nostr::Event =
        serde_json::from_slice(&bytes).map_err(|_| "invalid local possession proof")?;
    Ok(event.pubkey)
}

pub(crate) async fn http_header(
    state: &AppState,
    url: &str,
    auth: &str,
) -> Result<Option<reqwest::header::HeaderValue>, String> {
    let key = proof_key(auth)?;
    ensure(state, url, key).await?;
    session(state)?
        .header(url, key, now()?)
        .map_err(|error| error.to_string())
}

/// Call only on a no-redirect client. Never add this to shared default headers.
pub(crate) async fn authorize(
    state: &AppState,
    request: reqwest::RequestBuilder,
    url: &str,
    auth: &str,
) -> Result<reqwest::RequestBuilder, String> {
    Ok(match http_header(state, url, auth).await? {
        Some(header) => request.header(IDENTITY_HEADER, header),
        None => request,
    })
}

/// Cancel an operation and discard its result after login replacement or expiry.
/// Wrap body consumption too, so late responses cannot repopulate caches.
pub(crate) async fn guard<T>(
    state: &AppState,
    url: &str,
    key: PublicKey,
    operation: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    let identity = session(state)?;
    tokio::select! {
        biased;
        _ = identity.lease_ended(url, key) => Err("enterprise authentication changed; operation outcome may be unknown".into()),
        result = operation => result,
    }
}

/// Acquire/renew under one mutex so racing requests cannot overwrite newer tokens.
/// Credentials stay native; a key-specific assertion is requested for agent proofs.
pub(crate) async fn ensure(
    state: &AppState,
    destination: &str,
    key: PublicKey,
) -> Result<(), String> {
    let identity = session(state)?;
    if !identity.protects(destination).map_err(|e| e.to_string())? {
        return Ok(());
    }
    let generation = identity.generation().map_err(|e| e.to_string())?;
    let _single_flight = tokio::time::timeout(
        std::time::Duration::from_secs(35),
        state.federated_acquisition.lock(),
    )
    .await
    .map_err(|_| "enterprise assertion service busy; retry")?;
    if identity.generation().map_err(|e| e.to_string())? != generation {
        return Err("enterprise authentication changed".into());
    }
    if identity
        .expires_at(key)
        .map_err(|e| e.to_string())?
        .is_some_and(|exp| exp > now().unwrap_or(u64::MAX).saturating_add(60))
    {
        return Ok(());
    }
    if state
        .federated_retry_after
        .load(std::sync::atomic::Ordering::Acquire)
        > now()?
    {
        return Err("enterprise assertion service unavailable; retry shortly".into());
    }
    let app = state
        .app_handle
        .lock()
        .map_err(|_| "application unavailable")?
        .clone()
        .ok_or("application unavailable")?;
    let login = app.state::<crate::builderlab::BuilderlabSession>();
    let credential = login.credential_for_federated_identity()?;
    let human = state.signing_keys()?;
    let (keys, auth_tag) = if human.public_key() == key {
        (human.clone(), None)
    } else {
        let records = crate::managed_agents::load_managed_agents(&app)?;
        let record = records
            .iter()
            .find(|record| record.pubkey == key.to_hex())
            .ok_or("enterprise agent identity is not managed by this desktop")?;
        let keys = nostr::Keys::parse(&record.private_key_nsec)
            .map_err(|_| "agent signing identity unavailable")?;
        let auth = record
            .auth_tag
            .as_deref()
            .map(serde_json::from_str::<nostr::Tag>)
            .transpose()
            .map_err(|_| "invalid agent delegation")?;
        (keys, auth)
    };
    let active_relay = crate::relay::relay_ws_url_with_override(state);
    let mut target = url::Url::parse(destination).map_err(|_| "invalid enterprise destination")?;
    target.set_path("");
    target.set_query(None);
    let relay_url = target.as_str().trim_end_matches('/').to_string();
    let endpoint = option_env!("BUZZ_BUILD_NIP_FI_ASSERTION_URL")
        .unwrap_or("https://app.builderlab.xyz/api/goose/v1/buzz/identity/assertions");
    let result = buzz_ws_client_pkg::identity_adapter::exchange(
        &state.media_fetch_client,
        endpoint,
        &credential,
        &keys,
        &relay_url,
        auth_tag.as_ref(),
    )
    .await;
    let result = match result {
        Ok(result) => {
            state
                .federated_retry_after
                .store(0, std::sync::atomic::Ordering::Release);
            result
        }
        Err(error) => {
            state.federated_retry_after.store(
                now()?.saturating_add(10),
                std::sync::atomic::Ordering::Release,
            );
            return Err(error.to_string());
        }
    };
    if state.signing_keys()?.public_key() != human.public_key()
        || crate::relay::relay_ws_url_with_override(state) != active_relay
    {
        return Err("enterprise login scope changed".into());
    }
    identity
        .install(
            generation,
            result
                .into_assertion(key, now()?)
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())
}

/// Native sign-in gate. OSS communities pass through without corporate login.
#[tauri::command]
pub(crate) async fn acquire_federated_assertion(state: State<'_, AppState>) -> Result<(), String> {
    ensure(
        &state,
        &crate::relay::relay_ws_url_with_override(&state),
        state.signing_keys()?.public_key(),
    )
    .await
}

/// Credential-free build capability used by the sign-in screen.
#[tauri::command]
pub(crate) fn federated_identity_required(state: State<'_, AppState>) -> Result<bool, String> {
    session(&state)?
        .protects(&crate::relay::relay_ws_url_with_override(&state))
        .map_err(|e| e.to_string())
}

/// Refresh the primary assertion before expiry. Each failed attempt waits ten
/// seconds; current authority is never extended locally on adapter failures.
pub(crate) async fn renew_loop(app: tauri::AppHandle) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        let state = app.state::<AppState>();
        if state
            .shutdown_started
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return;
        }
        let Ok(key) = state.signing_keys().map(|keys| keys.public_key()) else {
            continue;
        };
        let Ok(identity) = session(&state) else {
            continue;
        };
        if identity.expires_at(key).ok().flatten().is_some() {
            let _ = ensure(
                &state,
                &crate::relay::relay_ws_url_with_override(&state),
                key,
            )
            .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn query_sink_sends_assertion_and_matching_local_proof() {
        use axum::{http::HeaderMap, routing::post, Router};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let keys = nostr::Keys::generate();
        let expected_key = keys.public_key();
        let app = Router::new().route(
            "/query",
            post(move |headers: HeaderMap, body: String| async move {
                assert_eq!(headers[IDENTITY_HEADER], "Bearer aaa.bbb.ccc");
                let auth = headers["authorization"].to_str().unwrap();
                assert_eq!(proof_key(auth).unwrap(), expected_key);
                assert_eq!(body, "[]");
                "[]"
            }),
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let mut state = crate::app_state::build_app_state();
        state.federated_identity = Ok(Arc::new(IdentitySession::new(&[&base]).unwrap()));
        let identity = session(&state).unwrap();
        identity
            .install(
                0,
                Assertion::new(
                    "aaa.bbb.ccc",
                    keys.public_key(),
                    now().unwrap() + 600,
                    now().unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
        let result = crate::relay::query_relay_at_with_keys(&state, &base, &[], &keys, None).await;
        server.abort();
        server.await.ok();
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn logout_discards_in_flight_http_result_and_requires_new_login() {
        let mut state = crate::app_state::build_app_state();
        let key = state.signing_keys().unwrap().public_key();
        state.federated_identity = Ok(Arc::new(
            IdentitySession::new(&["https://relay.example"]).unwrap(),
        ));
        let identity = session(&state).unwrap();
        identity
            .install(
                0,
                Assertion::new("a.b.c", key, now().unwrap() + 600, now().unwrap()).unwrap(),
            )
            .unwrap();
        let operation = guard(&state, "https://relay.example/query", key, async {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            Ok("must not reach cache")
        });
        let invalidate = async {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            identity.invalidate().unwrap();
        };
        let (result, ()) = tokio::join!(operation, invalidate);
        assert!(result.is_err());
        assert!(identity
            .header("https://relay.example/query", key, now().unwrap())
            .is_err());
    }

    #[test]
    fn derives_pairing_key_from_actual_local_proof() {
        let keys = nostr::Keys::generate();
        let auth = crate::relay::build_nip98_auth_header_for_keys(
            &keys,
            &reqwest::Method::POST,
            "https://relay.example/query",
            b"[]",
        )
        .unwrap();
        assert_eq!(proof_key(&auth).unwrap(), keys.public_key());
        assert!(proof_key("Bearer never-a-possession-proof").is_err());
    }
}
