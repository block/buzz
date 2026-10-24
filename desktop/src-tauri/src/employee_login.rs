//! Caller-bound Square employee login into the existing native BB session.
//! Only a verifier-bound one-use code crosses the browser callback boundary.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Deserialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri_plugin_opener::OpenerExt;
use tokio::{net::TcpListener, sync::oneshot};
use url::Url;

use crate::builderlab::{
    api_url_for_base, authenticated_user, BuilderlabAuthInfo, BuilderlabLogin, BuilderlabSession,
};

pub(crate) const API_BASE: &str = "https://square.builderlabstaging.xyz/api/goose";
const SQUARE_SIGN_IN: &str = "https://app.squareupstaging.com/dashboard/managerbot/buzz-sign-in";
const REDEEM_PATH: &str = "/v1/auth/merchant-team-member/native-handoff/redeem";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MAX_RESPONSE: usize = 64 * 1024;

/// The integration is disabled unless the operator explicitly opts in.
#[tauri::command]
pub(crate) fn employee_sign_in_available() -> bool {
    crate::employee_relay_policy::configured()
        .ok()
        .flatten()
        .is_some()
}

fn random_parameter() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|_| "Could not create secure sign-in transaction".to_owned())?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn is_parameter(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

struct CallbackState {
    nonce: String,
    state: String,
    sender: Mutex<Option<oneshot::Sender<String>>>,
}

async fn callback(
    Path(nonce): Path<String>,
    Query(params): Query<Vec<(String, String)>>,
    State(state): State<Arc<CallbackState>>,
) -> Response {
    let valid = params.len() == 2
        && params.iter().filter(|(key, _)| key == "code").count() == 1
        && params.iter().filter(|(key, _)| key == "state").count() == 1;
    let params: HashMap<_, _> = params.into_iter().collect();
    if !valid
        || nonce != state.nonce
        || params.get("state") != Some(&state.state)
        || !params.get("code").is_some_and(|code| is_parameter(code))
    {
        return (StatusCode::BAD_REQUEST, "Invalid sign-in callback").into_response();
    }
    let Ok(mut sender) = state.sender.lock() else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let Some(sender) = sender.take() else {
        return (StatusCode::CONFLICT, "Sign-in callback already received").into_response();
    };
    let Some(code) = params.get("code") else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let _ = sender.send(code.clone());
    ([ ("Cache-Control", "no-store"), ("Referrer-Policy", "no-referrer") ], Html("<!doctype html><html lang=\"en\"><title>Return to Buzz</title><h1>Return to Buzz</h1><p>Buzz will finish checking your Square sign-in. You can close this window.</p></html>")).into_response()
}

fn login_url(return_to: &str, state: &str, verifier: &str) -> Result<Url, String> {
    let mut url = Url::parse(SQUARE_SIGN_IN).map_err(|_| "Invalid Square sign-in configuration")?;
    url.query_pairs_mut()
        .append_pair("return_to", return_to)
        .append_pair("state", state)
        .append_pair(
            "code_challenge",
            &URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        )
        .append_pair("code_challenge_method", "S256");
    Ok(url)
}

struct StopListener(tokio::task::JoinHandle<()>);
impl Drop for StopListener {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RedeemedSession {
    session_credential: String,
    expires_at: String,
    subject_id: String,
    org: String,
    workspace_identifier: String,
}

async fn read_response<T: DeserializeOwned>(mut response: reqwest::Response) -> Result<T, String> {
    if !response.status().is_success() {
        return Err(format!(
            "Square sign-in refused (HTTP {})",
            response.status()
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not read Square sign-in response")?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE {
            return Err("Square sign-in response is too large".to_owned());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid Square sign-in response".to_owned())
}

fn verify_session(
    receipt: &RedeemedSession,
    subject: Option<&str>,
    expiry: &str,
) -> Result<(), String> {
    let receipt_expiry = DateTime::parse_from_rfc3339(&receipt.expires_at)
        .map_err(|_| "Invalid Square session expiry")?;
    let checked_expiry =
        DateTime::parse_from_rfc3339(expiry).map_err(|_| "Invalid Square session expiry")?;
    if receipt.session_credential.is_empty()
        || receipt.subject_id.is_empty()
        || receipt.org.is_empty()
        || receipt.workspace_identifier.is_empty()
        || subject != Some(receipt.subject_id.as_str())
        || receipt_expiry != checked_expiry
        || receipt_expiry <= Utc::now()
    {
        return Err("Square session binding did not match".to_owned());
    }
    Ok(())
}

/// Opens Square's employee session flow and commits only the exact verified CTE session.
#[tauri::command]
pub(crate) async fn start_employee_login(
    app: tauri::AppHandle,
    app_state: tauri::State<'_, crate::app_state::AppState>,
    session: tauri::State<'_, BuilderlabSession>,
    login: tauri::State<'_, BuilderlabLogin>,
) -> Result<BuilderlabAuthInfo, String> {
    if !employee_sign_in_available() {
        return Err("Square employee sign-in is not enabled".to_owned());
    }
    let selected = crate::relay::relay_ws_url_with_override(&app_state);
    let relay = crate::employee_relay_policy::relay_for_url(&selected)?
        .ok_or("Select a configured Square community before signing in")?;
    if selected != relay {
        return Err("Select the configured Square community before signing in".to_owned());
    }
    let workspace_generation = app_state
        .workspace_apply_generation
        .load(std::sync::atomic::Ordering::Acquire);
    let pubkey = app_state.signing_keys()?.public_key().to_hex();
    let verifier = random_parameter()?;
    let state = random_parameter()?;
    let nonce = random_parameter()?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| "Could not start local sign-in listener")?;
    let port = listener
        .local_addr()
        .map_err(|_| "Could not read local sign-in listener")?
        .port();
    let return_to = format!("http://127.0.0.1:{port}/callback/{nonce}");
    let (sender, receiver) = oneshot::channel();
    let router = Router::new()
        .route("/callback/{nonce}", get(callback))
        .with_state(Arc::new(CallbackState {
            nonce,
            state: state.clone(),
            sender: Mutex::new(Some(sender)),
        }));
    let server = StopListener(tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    }));
    let (login_id, mut cancel) = login.begin_employee_login()?;
    let url = login_url(&return_to, &state, &verifier)?;
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|_| "Could not open Square sign-in")?;
    let code = tokio::select! {
        result = tokio::time::timeout(LOGIN_TIMEOUT,receiver) => result.map_err(|_| "Square sign-in timed out")?.map_err(|_| "Square sign-in listener stopped")?,
        _ = &mut cancel => return Err("Square sign-in canceled".to_owned()),
    };
    drop(server);
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "Could not configure Square sign-in")?;
    let request = crate::employee_api::request(client.post(api_url_for_base(API_BASE, REDEEM_PATH)?).json(&serde_json::json!({"code":code,"code_verifier":verifier,"return_to":return_to,"state":state})))?;
    let response = client
        .execute(request)
        .await
        .map_err(|_| "Could not exchange Square sign-in code")?;
    let receipt: RedeemedSession = read_response(response).await?;
    let me = authenticated_user(&client, &receipt.session_credential, API_BASE).await?;
    verify_session(&receipt, me.subject.as_deref(), &me.expires_at)?;
    let _identity_guard = app_state
        .identity_mutation
        .lock()
        .map_err(|_| "Buzz identity changed")?;
    if app_state.signing_keys()?.public_key().to_hex() != pubkey {
        return Err("Buzz identity changed. Sign in again.".to_owned());
    }
    if app_state
        .workspace_apply_generation
        .load(std::sync::atomic::Ordering::Acquire)
        != workspace_generation
        || crate::relay::relay_ws_url_with_override(&app_state) != relay
    {
        return Err("Community changed. Sign in again.".to_owned());
    }
    let expires_at = DateTime::parse_from_rfc3339(&receipt.expires_at)
        .map_err(|_| "Invalid Square session expiry")?
        .timestamp();
    session.finish_employee_login(
        &login,
        login_id,
        receipt.session_credential.clone(),
        API_BASE,
        crate::builderlab::EmployeeBinding {
            pubkey,
            expires_at,
            relay: relay.clone(),
        },
    )?;
    drop(_identity_guard);
    let keys = app_state.signing_keys()?;
    // The existing key-bound NIP-FI issuance atomically verifies and enrolls this own key.
    let (result, canceled) = tokio::select! {
        biased;
        _ = &mut cancel => (Err("Square sign-in canceled".to_owned()), true),
        result = crate::builderlab::employee_evidence(&app_state, &keys, &relay) => (result.map(|_| ()), false),
    };
    if let Err(error) = result {
        session.clear_failed_employee_login(&receipt.session_credential, canceled)?;
        let _ = login.complete_employee_login(login_id);
        crate::builderlab::notify_session_changed(&app_state);
        return Err(error);
    }
    if let Err(error) = login.complete_employee_login(login_id) {
        session.clear_if_credential(&receipt.session_credential)?;
        crate::builderlab::notify_session_changed(&app_state);
        return Err(error);
    }
    crate::builderlab::notify_session_changed(&app_state);
    Ok(BuilderlabAuthInfo {
        expires_at: me.expires_at,
        email: None,
        name: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_url_contains_only_public_transaction_data() {
        let verifier = "v".repeat(43);
        let url = login_url(
            "http://127.0.0.1:12345/callback/nonce",
            &"s".repeat(43),
            &verifier,
        )
        .unwrap();
        assert_eq!(url.host_str(), Some("app.squareupstaging.com"));
        assert!(!url.as_str().contains(&verifier));
        assert!(!url.as_str().contains("session_credential"));
        let params: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(
            params.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        assert_eq!(
            params.get("code_challenge"),
            Some(&URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())))
        );
    }
    #[tokio::test]
    async fn callback_rejects_state_and_duplicate_parameters_without_consuming() {
        let (sender, mut receiver) = oneshot::channel();
        let state = Arc::new(CallbackState {
            nonce: "nonce".to_owned(),
            state: "state".to_owned(),
            sender: Mutex::new(Some(sender)),
        });
        let code = "c".repeat(43);
        for pairs in [
            vec![
                ("code".to_owned(), code.clone()),
                ("state".to_owned(), "wrong".to_owned()),
            ],
            vec![
                ("code".to_owned(), code.clone()),
                ("code".to_owned(), code.clone()),
                ("state".to_owned(), "state".to_owned()),
            ],
        ] {
            assert_eq!(
                callback(Path("nonce".to_owned()), Query(pairs), State(state.clone()))
                    .await
                    .status(),
                StatusCode::BAD_REQUEST
            );
            assert!(receiver.try_recv().is_err());
        }
        let pairs = vec![
            ("code".to_owned(), code.clone()),
            ("state".to_owned(), "state".to_owned()),
        ];
        assert_eq!(
            callback(
                Path("nonce".to_owned()),
                Query(pairs.clone()),
                State(state.clone())
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(receiver.await.unwrap(), code);
        assert_eq!(
            callback(Path("nonce".to_owned()), Query(pairs), State(state))
                .await
                .status(),
            StatusCode::CONFLICT
        );
    }
    #[tokio::test]
    async fn native_loopback_runs_through_real_http_query_extraction() {
        let (sender, receiver) = oneshot::channel();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(CallbackState {
            nonce: "nonce".to_owned(),
            state: "state".to_owned(),
            sender: Mutex::new(Some(sender)),
        });
        let router = Router::new()
            .route("/callback/{nonce}", get(callback))
            .with_state(state);
        let server = StopListener(tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        }));
        let response = reqwest::Client::new()
            .get(format!(
                "http://127.0.0.1:{port}/callback/nonce?code={}&state=state",
                "c".repeat(43)
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(receiver.await.unwrap(), "c".repeat(43));
        drop(server);
    }
    #[test]
    fn receipt_requires_same_subject_and_expiry_instant() {
        let expiry = (Utc::now() + chrono::Duration::minutes(4)).to_rfc3339();
        let receipt = RedeemedSession {
            session_credential: "secret".to_owned(),
            expires_at: expiry.clone(),
            subject_id: "person".to_owned(),
            org: "org".to_owned(),
            workspace_identifier: "workspace".to_owned(),
        };
        assert!(verify_session(&receipt, Some("person"), &expiry).is_ok());
        assert!(verify_session(&receipt, Some("other"), &expiry).is_err());
        assert!(verify_session(
            &receipt,
            Some("person"),
            &(Utc::now() + chrono::Duration::hours(8)).to_rfc3339()
        )
        .is_err());
    }
}

/// Public session state for the pre-connection employee login gate.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EmployeeSessionStatus {
    required: bool,
    authenticated: bool,
    can_retry: bool,
    expires_at: Option<i64>,
}

#[tauri::command]
pub(crate) fn employee_session_status(
    app_state: tauri::State<'_, crate::app_state::AppState>,
    session: tauri::State<'_, BuilderlabSession>,
) -> Result<EmployeeSessionStatus, String> {
    let relay = crate::relay::relay_ws_url_with_override(&app_state);
    if !crate::builderlab::requires_employee(&relay)? {
        return Ok(EmployeeSessionStatus {
            required: false,
            authenticated: false,
            can_retry: false,
            expires_at: None,
        });
    }
    let expires_at = session.employee_expiry(&app_state);
    Ok(EmployeeSessionStatus {
        required: true,
        authenticated: expires_at.is_some(),
        can_retry: session.employee_can_retry(&app_state),
        expires_at,
    })
}

/// Explicitly rechecks the current employee's key and membership before reconnecting.
#[tauri::command]
pub(crate) async fn refresh_employee_relay_access(
    app_state: tauri::State<'_, crate::app_state::AppState>,
    session: tauri::State<'_, BuilderlabSession>,
) -> Result<(), String> {
    let keys = app_state.signing_keys()?;
    let relay = crate::relay::relay_ws_url_with_override(&app_state);
    if !crate::builderlab::requires_employee(&relay)? {
        return Err("Select the configured Square community".to_owned());
    }
    session
        .evidence_with_refresh(&app_state, &keys, &relay, true)
        .await?;
    crate::builderlab::notify_session_changed(&app_state);
    Ok(())
}
