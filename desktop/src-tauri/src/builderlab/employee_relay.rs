//! Exact configured-relay transport. Session and assertion credentials remain native.
use super::*;
use nostr::Keys;
use tauri::Manager;
use tokio_util::sync::CancellationToken;

pub(crate) struct EmployeeEvidence {
    pub(crate) header: reqwest::header::HeaderValue,
    pub(crate) cancel: CancellationToken,
    pub(crate) expires_at: i64,
}

pub(crate) fn requires_employee(value: &str) -> Result<bool, String> {
    let Some(configured) = crate::employee_login::configured_relay()? else {
        return Ok(false);
    };
    let parsed = Url::parse(value).map_err(|_| "Invalid relay URL")?;
    let authority = &parsed[url::Position::BeforeHost..url::Position::AfterPort];
    if format!("wss://{authority}") != configured {
        return Ok(false);
    }
    if !matches!(parsed.scheme(), "https" | "wss")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err("Employee relay requires its exact TLS origin".to_owned());
    }
    Ok(true)
}

/// Refuses operator sessions and other signing keys on the configured employee relay.
pub(crate) async fn employee_evidence(
    app_state: &crate::app_state::AppState,
    keys: &Keys,
    relay_or_url: &str,
) -> Result<Option<EmployeeEvidence>, String> {
    if !requires_employee(relay_or_url)? {
        return Ok(None);
    }
    let configured =
        crate::employee_login::configured_relay()?.ok_or("Square relay is not configured")?;
    if crate::relay::relay_ws_url_with_override(app_state) != configured
        || app_state.signing_keys()?.public_key() != keys.public_key()
    {
        return Err("Employee community or Buzz identity changed".to_owned());
    }
    let handle = app_state
        .app_handle
        .lock()
        .map_err(|_| "Identity service unavailable")?
        .clone()
        .ok_or("Identity service unavailable")?;
    let session = handle.state::<BuilderlabSession>();
    session
        .evidence(app_state, keys, &configured)
        .await
        .map(Some)
}

impl BuilderlabSession {
    async fn evidence(
        &self,
        app_state: &crate::app_state::AppState,
        keys: &Keys,
        relay: &str,
    ) -> Result<EmployeeEvidence, String> {
        self.evidence_with_refresh(app_state, keys, relay, false)
            .await
    }

    pub(crate) async fn evidence_with_refresh(
        &self,
        app_state: &crate::app_state::AppState,
        keys: &Keys,
        relay: &str,
        force: bool,
    ) -> Result<EmployeeEvidence, String> {
        let _issuance = self.1.lock().await;
        if force {
            self.invalidate_employee_assertion()?;
            let mut stored = self.0.lock().map_err(|_| "Square session unavailable")?;
            if let Some(stored) = stored.as_mut() {
                if stored.refusal.take().is_some() {
                    stored.cancel = CancellationToken::new();
                }
            }
        }
        if crate::relay::relay_ws_url_with_override(app_state) != relay {
            return Err("Employee community changed".to_owned());
        }
        let (credential, base) = self
            .request_context(app_state)?
            .ok_or("Sign in with Square for this community")?;
        let generation = app_state
            .workspace_apply_generation
            .load(std::sync::atomic::Ordering::Acquire);
        let (expires_at, cancel) = {
            let stored = self.0.lock().map_err(|_| "Square session unavailable")?;
            let stored = stored.as_ref().ok_or("Sign in with Square")?;
            if let Some(reason) = &stored.refusal {
                return Err(reason.clone());
            }
            let binding = stored
                .employee
                .as_ref()
                .ok_or("A Square employee session is required")?;
            if binding.pubkey != keys.public_key().to_hex() {
                return Err("Square session does not authorize this Buzz key".to_owned());
            }
            if let Some(assertion) = stored
                .assertion
                .as_ref()
                .filter(|value| value.expires_at > chrono::Utc::now().timestamp() + 10)
            {
                return evidence(
                    &assertion.value,
                    assertion.expires_at,
                    stored.cancel.clone(),
                );
            }
            (binding.expires_at, stored.cancel.clone())
        };
        let url = api_url_for_base(base, "/v1/buzz/identity/assertions")?;
        let body = serde_json::to_vec(
            &serde_json::json!({"relay_url":relay,"nostr_pubkey":keys.public_key().to_hex()}),
        )
        .map_err(|_| "Invalid assertion request")?;
        let proof = crate::relay::build_nip98_auth_header_for_keys(
            keys,
            &reqwest::Method::POST,
            url.as_str(),
            &body,
        )?;
        let request = app_state
            .media_fetch_client
            .post(url)
            .header(BB_SESSION_CREDENTIAL_HEADER, session_header(&credential)?)
            .header("Authorization", proof)
            .header("Content-Type", "application/json")
            .body(body)
            .timeout(Duration::from_secs(30));
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err("Square sign-in changed".to_owned()),
            result = request.send() => result.map_err(|_| "Could not request employee relay access")?,
        };
        let status = response.status();
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = tokio::select! {
            _ = cancel.cancelled() => return Err("Square sign-in changed".to_owned()),
            result = response.chunk() => result.map_err(|_| "Could not read relay authorization")?,
        } {
            if bytes.len() + chunk.len() > 32 * 1024 {
                return Err("Relay authorization response is too large".to_owned());
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            let code = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|value| {
                    value
                        .get("error")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                });
            match code.as_deref() {
                Some("session_required" | "session_expired") => {
                    self.clear_if_credential(&credential)?;
                    super::notify_session_changed(app_state);
                }
                Some("binding_mismatch" | "authorization_denied") => {
                    self.refuse_employee_access(&credential, "Square access is suspended. Use Retry community connection after your employer resolves access.".to_owned())?;
                    super::notify_session_changed(app_state);
                }
                _ => {}
            }
            return Err(match code.as_deref() {
                Some("binding_mismatch") => "This Square account is linked to another Buzz key. Use that Buzz identity or ask your employer to resolve the link.",
                Some("authorization_denied") => "Square access is not prepared or has been disabled. Ask your employer to check your access.",
                Some("session_required" | "session_expired") => "Your Square session expired. Sign in again.",
                Some("rate_limited") => "Too many access attempts. Wait a moment and retry.",
                Some("invalid_proof") => "Could not verify this Buzz key. Check your device clock and retry.",
                _ => "Square access is temporarily unavailable. Retry in a moment.",
            }.to_owned());
        }
        #[derive(Deserialize)]
        struct Receipt {
            assertion: String,
            nostr_pubkey: String,
            expires_at: i64,
        }
        let receipt: Receipt =
            serde_json::from_slice(&bytes).map_err(|_| "Invalid relay authorization response")?;
        let now = chrono::Utc::now().timestamp();
        if receipt.nostr_pubkey != keys.public_key().to_hex()
            || receipt.expires_at <= now
            || receipt.expires_at > now + 300
            || receipt.expires_at > expires_at
            || cancel.is_cancelled()
            || app_state
                .workspace_apply_generation
                .load(std::sync::atomic::Ordering::Acquire)
                != generation
            || crate::relay::relay_ws_url_with_override(app_state) != relay
            || app_state.signing_keys()?.public_key() != keys.public_key()
        {
            return Err("Employee relay authorization changed".to_owned());
        }
        let result = evidence(&receipt.assertion, receipt.expires_at, cancel)?;
        let mut stored = self.0.lock().map_err(|_| "Square session unavailable")?;
        let stored = stored
            .as_mut()
            .filter(|stored| stored.credential == credential)
            .ok_or("Square session changed")?;
        stored.assertion = Some(CachedAssertion {
            value: receipt.assertion,
            expires_at: receipt.expires_at,
        });
        Ok(result)
    }
}
fn evidence(
    value: &str,
    expires_at: i64,
    cancel: CancellationToken,
) -> Result<EmployeeEvidence, String> {
    if value.len() > 16 * 1024
        || (value.split('.').count() != 3 || value.split('.').any(str::is_empty))
        || value
            .bytes()
            .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')))
    {
        return Err("Invalid relay assertion".to_owned());
    }
    let mut header = reqwest::header::HeaderValue::from_str(&format!("Bearer {value}"))
        .map_err(|_| "Invalid relay assertion")?;
    header.set_sensitive(true);
    Ok(EmployeeEvidence {
        header,
        cancel,
        expires_at,
    })
}

/// Ordinary public media keeps its existing behavior, including identity recovery mode.
pub(crate) async fn send_employee_media_request(
    state: &crate::app_state::AppState,
    url: &str,
    request: reqwest::RequestBuilder,
) -> Result<reqwest::Response, String> {
    if !requires_employee(url)? {
        return request
            .send()
            .await
            .map_err(|error| crate::relay::classify_request_error(&error));
    }
    let keys = state.signing_keys()?;
    send_employee_request(state, &keys, url, request).await
}

/// Adds only scoped relay evidence. The request's own NIP-98 proof is preserved.
pub(crate) async fn send_employee_request(
    app_state: &crate::app_state::AppState,
    keys: &Keys,
    url: &str,
    request: reqwest::RequestBuilder,
) -> Result<reqwest::Response, String> {
    if !requires_employee(url)? {
        return request
            .send()
            .await
            .map_err(|error| crate::relay::classify_request_error(&error));
    }
    let (_, request) = request.build_split();
    let mut request = request.map_err(|_| "Invalid employee relay request")?;
    let timeout = request
        .timeout()
        .copied()
        .unwrap_or(Duration::from_secs(30));
    let started = tokio::time::Instant::now();
    tokio::time::timeout(timeout, async {
        let evidence = employee_evidence(app_state, keys, url)
            .await?
            .ok_or("Employee relay evidence is required")?;
        refresh_nip98_request(&mut request, keys)?;
        // Keep response-body consumption inside the original operation budget too.
        *request.timeout_mut() = Some(timeout.saturating_sub(started.elapsed()));
        // This client never follows redirects, including media request builders.
        send_authorized_request(app_state.media_fetch_client.clone(), request, evidence).await
    })
    .await
    .map_err(|_| "relay unreachable: request timed out".to_owned())?
}

fn refresh_nip98_request(request: &mut reqwest::Request, keys: &Keys) -> Result<(), String> {
    use base64::Engine;
    let encoded = request
        .headers()
        .get("Authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Nostr "));
    let nip98 = encoded
        .and_then(|value| {
            base64::engine::general_purpose::STANDARD
                .decode(value)
                .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(value))
                .ok()
        })
        .and_then(|value| serde_json::from_slice::<serde_json::Value>(&value).ok())
        .is_some_and(|value| value["kind"].as_u64() == Some(27235));
    if nip98 {
        let body = match request.body() {
            Some(body) => body
                .as_bytes()
                .ok_or("Cannot sign a streaming NIP-98 request")?,
            None => &[],
        };
        let proof = crate::relay::build_nip98_auth_header_for_keys(
            keys,
            request.method(),
            request.url().as_str(),
            body,
        )?;
        request.headers_mut().insert(
            "Authorization",
            proof.parse().map_err(|_| "Invalid relay proof")?,
        );
    }
    Ok(())
}

async fn send_authorized_request(
    client: reqwest::Client,
    mut request: reqwest::Request,
    evidence: EmployeeEvidence,
) -> Result<reqwest::Response, String> {
    let assertion = evidence
        .header
        .to_str()
        .map_err(|_| "Invalid relay assertion")?
        .trim_start_matches("Bearer ")
        .to_owned();
    request
        .headers_mut()
        .insert("Nostr-Federated-Identity", evidence.header);
    let remaining = Duration::from_secs(
        evidence
            .expires_at
            .saturating_sub(chrono::Utc::now().timestamp())
            .max(0) as u64,
    );
    let mut response = tokio::select! {
        biased;
        _ = evidence.cancel.cancelled()=>return Err("Square sign-in changed".to_owned()),
        _ = tokio::time::sleep(remaining)=>return Err("Employee relay authorization expired".to_owned()),
        response=client.execute(request)=>response.map_err(|error|crate::relay::classify_request_error(&error))?,
    };
    response.extensions_mut().insert(ProtectedAssertion(
        assertion,
        evidence.cancel,
        evidence.expires_at,
    ));
    super::employee_response::filter_response(response)
}

#[cfg(test)]
mod tests;
