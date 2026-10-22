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
        let _issuance = self.1.lock().await;
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
            return Err(match code.as_deref() {
                Some("binding_mismatch") => "This Square account is linked to another Buzz key. Use that Buzz identity or ask your employer to resolve the link.",
                Some("authorization_denied") => "Square access is not prepared or has been disabled. Ask your employer to check your access.",
                Some("session_expired") => "Your Square session expired. Sign in again.",
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

/// Adds only scoped relay evidence. The request's own NIP-98 proof is preserved.
pub(crate) async fn send_employee_request(
    app_state: &crate::app_state::AppState,
    keys: &Keys,
    url: &str,
    request: reqwest::RequestBuilder,
) -> Result<reqwest::Response, String> {
    let Some(evidence) = employee_evidence(app_state, keys, url).await? else {
        return request
            .send()
            .await
            .map_err(|error| crate::relay::classify_request_error(&error));
    };
    send_authorized_request(request, evidence).await
}

async fn send_authorized_request(
    request: reqwest::RequestBuilder,
    evidence: EmployeeEvidence,
) -> Result<reqwest::Response, String> {
    tokio::select! {
        _ = evidence.cancel.cancelled()=>Err("Square sign-in changed".to_owned()),
        response=request.header("Nostr-Federated-Identity",evidence.header).send()=>response.map_err(|_|"Employee relay request failed".to_owned()),
    }
}

#[cfg(test)]
mod tests;
