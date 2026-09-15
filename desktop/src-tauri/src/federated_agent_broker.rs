//! Loopback assertion broker for desktop-owned agents. Per-key capabilities are
//! ephemeral and login-generation-bound; the corporate session never leaves Buzz.
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU16, Ordering},
        Mutex,
    },
};
use tauri::Manager;

#[derive(Default)]
pub(crate) struct AgentBroker {
    port: AtomicU16,
    grants: Mutex<HashMap<String, Grant>>,
}
struct Grant {
    key: String,
    relay: String,
    generation: u64,
}

/// Construct reserved launch env only, never persisted to records or snapshots.
pub(crate) fn launch_env(
    app: &tauri::AppHandle,
    key: &str,
    relay: &str,
) -> Result<Vec<(String, String)>, String> {
    let state = app.state::<crate::app_state::AppState>();
    let identity = crate::federated_identity::session(&state)?;
    if !identity.protects(relay).map_err(|e| e.to_string())? {
        return Ok(vec![]);
    }
    // Refuse launch until the owner's login is live; issuance still checks agent policy.
    app.state::<crate::builderlab::BuilderlabSession>()
        .credential_for_federated_identity()?;
    let generation = identity.generation().map_err(|e| e.to_string())?;
    let port = state.federated_broker.port.load(Ordering::Acquire);
    if port == 0 {
        return Err("enterprise agent broker is starting; retry".into());
    }
    let mut grants = state
        .federated_broker
        .grants
        .lock()
        .map_err(|_| "enterprise broker unavailable")?;
    grants.retain(|_, g| g.generation == generation);
    if grants.len() >= 256 {
        return Err("enterprise agent broker capacity reached".into());
    }
    let capability = grants
        .iter()
        .find(|(_, g)| g.key == key && g.relay == relay)
        .map(|(secret, _)| secret.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
    grants.insert(
        capability.clone(),
        Grant {
            key: key.into(),
            relay: relay.into(),
            generation,
        },
    );
    Ok(vec![
        (
            "BUZZ_NIP_FI_ENDPOINT".into(),
            format!("http://127.0.0.1:{port}/assertions"),
        ),
        ("BUZZ_NIP_FI_CREDENTIAL".into(), capability),
        ("BUZZ_NIP_FI_ORIGINS".into(), relay.into()),
    ])
}

#[derive(Deserialize)]
struct Request {
    nostr_pubkey: String,
    relay_url: String,
}

async fn handle(
    State(app): State<tauri::AppHandle>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    match issue(&app, &headers, &body).await {
        Ok(value) => ([("cache-control", "no-store")], Json(value)).into_response(),
        Err(_) => (StatusCode::UNAUTHORIZED, "enterprise sign-in required").into_response(),
    }
}

async fn issue(
    app: &tauri::AppHandle,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<serde_json::Value, String> {
    // Browser pages may not drive a credential broker, even from the Tauri origin.
    if headers.contains_key("origin") {
        return Err("browser request denied".into());
    }
    let state = app.state::<crate::app_state::AppState>();
    let identity = crate::federated_identity::session(&state)?;
    let request: Request = serde_json::from_slice(body).map_err(|_| "invalid request")?;
    let generation = identity.generation().map_err(|e| e.to_string())?;
    let secret = headers
        .get("x-bb-session-credential")
        .and_then(|v| v.to_str().ok())
        .ok_or("missing capability")?;
    {
        let grants = state
            .federated_broker
            .grants
            .lock()
            .map_err(|_| "broker unavailable")?;
        let grant = grants.get(secret).ok_or("invalid capability")?;
        if grant.key != request.nostr_pubkey
            || grant.relay != request.relay_url
            || grant.generation != generation
        {
            return Err("stale capability".into());
        }
    }
    let encoded = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Nostr "))
        .ok_or("missing proof")?;
    let proof: nostr::Event =
        serde_json::from_slice(&STANDARD.decode(encoded).map_err(|_| "invalid proof")?)
            .map_err(|_| "invalid proof")?;
    proof.verify().map_err(|_| "invalid proof")?;
    let now = crate::federated_identity::now()?;
    let endpoint = format!(
        "http://127.0.0.1:{}/assertions",
        state.federated_broker.port.load(Ordering::Acquire)
    );
    let one_tag = |name: &str, expected: &str| {
        let tags: Vec<_> = proof
            .tags
            .iter()
            .filter(|t| t.as_slice().first().is_some_and(|s| s == name))
            .collect();
        tags.len() == 1 && tags[0].as_slice().len() == 2 && tags[0].as_slice()[1] == expected
    };
    if proof.pubkey.to_hex() != request.nostr_pubkey
        || proof.kind.as_u16() != 27235
        || proof.created_at.as_secs() > now.saturating_add(5)
        || now.saturating_sub(proof.created_at.as_secs()) > 60
        || !one_tag("u", &endpoint)
        || !one_tag("method", "POST")
        || !one_tag("payload", &hex::encode(Sha256::digest(body)))
    {
        return Err("invalid proof".into());
    }
    app.state::<crate::builderlab::BuilderlabSession>()
        .credential_for_federated_identity()?;
    if proof.pubkey != state.signing_keys()?.public_key()
        && !crate::managed_agents::load_managed_agents(app)?
            .iter()
            .any(|r| r.pubkey == request.nostr_pubkey)
    {
        return Err("agent no longer managed".into());
    }
    crate::federated_identity::ensure(&state, &request.relay_url, proof.pubkey).await?;
    if identity.generation().map_err(|e| e.to_string())? != generation {
        return Err("login changed".into());
    }
    let header = identity
        .header(&request.relay_url, proof.pubkey, now)
        .map_err(|e| e.to_string())?
        .ok_or("enterprise origin required")?;
    let token = header
        .to_str()
        .map_err(|_| "invalid assertion")?
        .strip_prefix("Bearer ")
        .ok_or("invalid assertion")?;
    Ok(
        serde_json::json!({"assertion": token, "nostr_pubkey": request.nostr_pubkey,
        "expires_at": identity.expires_at(proof.pubkey).map_err(|e| e.to_string())?}),
    )
}

pub(crate) async fn run(app: tauri::AppHandle) {
    let Ok(listener) = tokio::net::TcpListener::bind("127.0.0.1:0").await else {
        return;
    };
    let Ok(address) = listener.local_addr() else {
        return;
    };
    app.state::<crate::app_state::AppState>()
        .federated_broker
        .port
        .store(address.port(), Ordering::Release);
    let router = Router::new()
        .route("/assertions", post(handle))
        .layer(DefaultBodyLimit::max(24 * 1024))
        .with_state(app.clone());
    let _ = axum::serve(listener, router).await;
    app.state::<crate::app_state::AppState>()
        .federated_broker
        .port
        .store(0, Ordering::Release);
}
