//! Twilio inbound SMS webhook — `POST /hooks/sms/inbound`.
//!
//! Trust chain for an inbound SMS, in order, each step fatal on failure:
//! 1. `X-Twilio-Signature` validates the request actually came from Twilio
//!    ([`crate::twilio_auth::validate_signature`]).
//! 2. The sender's phone number must be `allowed = true` in `sms_identities`
//!    (closes the anonymous-spam / oracle vector — an unlisted number gets
//!    the exact same rejection as a signature failure, so a prober can't
//!    distinguish "bad signature" from "signature fine, number not allowed").
//!
//! This slice stops at "allowed → 200 OK, acknowledged". Turning an allowed
//! message into a `KIND_STREAM_MESSAGE_V2` relay event, and the sms-operator
//! persona that reads it, are follow-up slices.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Form, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;

use crate::state::AppState;
use crate::twilio_auth::validate_signature;

use super::{api_error, internal_error};

/// Generic rejection for both "bad signature" and "number not allowed" —
/// deliberately identical so the endpoint isn't an oracle for either fact.
fn rejected() -> (StatusCode, Json<serde_json::Value>) {
    api_error(StatusCode::FORBIDDEN, "request not accepted")
}

/// Handle an inbound Twilio SMS webhook POST. Validates the request
/// signature, then requires the sender's phone number be allow-listed;
/// on success, currently just acknowledges (event synthesis is a follow-up).
pub async fn twilio_inbound(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(params): Form<BTreeMap<String, String>>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    let (Some(auth_token), Some(webhook_url)) = (
        state.config.twilio_auth_token.as_deref(),
        state.config.twilio_webhook_url.as_deref(),
    ) else {
        // Not configured — fail closed rather than skip validation.
        return Err(rejected());
    };

    let signature = headers
        .get("X-Twilio-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !validate_signature(webhook_url, &params, auth_token, signature) {
        return Err(rejected());
    }

    let from = params.get("From").map(String::as_str).unwrap_or("");
    if from.is_empty() {
        return Err(rejected());
    }

    let identity = state
        .db
        .get_sms_identity(from)
        .await
        .map_err(|_| internal_error("sms identity lookup failed"))?;

    if !identity.is_some_and(|i| i.allowed) {
        return Err(rejected());
    }

    // TODO(slice 8+): synthesize a KIND_STREAM_MESSAGE_V2 event into the
    // community's SMS-inbox channel here instead of just acknowledging.
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({ "status": "accepted" })),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_is_forbidden_and_generic() {
        let (status, Json(body)) = rejected();
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "request not accepted");
    }
}
