//! Narrow, owner-visible agent-management requests for Fizz.
//!
//! These tools never write Desktop's agent store. They send an encrypted,
//! owner-scoped observer frame; Desktop validates that the sender is its Fizz
//! instance, opens a review surface, and performs the existing confirmed write.

use buzz_core::observer::{encrypt_observer_payload, OBSERVER_FRAME_TELEMETRY};
use buzz_sdk::{build_agent_observer_frame, nip_oa};
use buzz_ws_client::publish_event;
use rmcp::{model::CallToolResult, ErrorData};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const REQUEST_KIND: &str = "fizz_agent_management_request";
const OBSERVER_EVENT_KIND: &str = "fizz_agent_management_request";
const MAX_NAME_CHARS: usize = 120;
const MAX_PROMPT_CHARS: usize = 20_000;
const MAX_RATIONALE_CHARS: usize = 2_000;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CreateAgentParams {
    /// The channel UUID where the new agent should be added after confirmation.
    pub channel_id: String,
    /// The agent's human-readable name.
    pub display_name: String,
    /// Instructions that define the agent's role and behavior.
    pub system_prompt: String,
    /// Why this agent is useful, shown to the owner in the review UI.
    pub rationale: String,
    /// Optional runtime ID suggested by Fizz (for example `buzz-agent`).
    #[serde(default)]
    pub runtime: Option<String>,
    /// Optional provider suggested by Fizz. Credentials are never accepted here.
    #[serde(default)]
    pub provider: Option<String>,
    /// Optional model ID suggested by Fizz.
    #[serde(default)]
    pub model: Option<String>,
    /// Who may invoke the agent: `owner-only` or `anyone`. Allowlist changes
    /// stay in Desktop because they require selecting real people, not chat text.
    #[serde(default)]
    pub respond_to: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct UpdateAgentParams {
    /// The channel UUID where this conversation with Fizz is taking place.
    pub channel_id: String,
    /// The current display name of the reusable personal agent to update.
    /// Desktop resolves it locally so profile IDs never have to enter chat.
    pub agent_name: String,
    /// A concise explanation of the requested changes, shown in review.
    pub rationale: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub respond_to: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagementRequest<T> {
    #[serde(rename = "type")]
    request_type: &'static str,
    action: &'static str,
    request_id: String,
    request: T,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ObserverEvent<T> {
    seq: u64,
    timestamp: String,
    kind: &'static str,
    agent_index: Option<usize>,
    channel_id: Option<String>,
    session_id: Option<String>,
    turn_id: Option<String>,
    payload: ManagementRequest<T>,
}

fn text_result(text: impl Into<String>) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text.into(),
    )]))
}

fn trim_required(value: &str, label: &str, max: usize) -> Result<String, ErrorData> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ErrorData::invalid_params(
            format!("{label} is required"),
            None,
        ));
    }
    if value.chars().count() > max {
        return Err(ErrorData::invalid_params(
            format!("{label} is too long (max {max} characters)"),
            None,
        ));
    }
    Ok(value.to_owned())
}

fn validate_optional(value: Option<String>, label: &str) -> Result<Option<String>, ErrorData> {
    match value {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() || trimmed.chars().count() > 300 {
                return Err(ErrorData::invalid_params(
                    format!("{label} must be between 1 and 300 characters"),
                    None,
                ));
            }
            Ok(Some(trimmed.to_owned()))
        }
        None => Ok(None),
    }
}

fn validate_respond_to(value: Option<String>) -> Result<Option<String>, ErrorData> {
    match value.as_deref() {
        None => Ok(None),
        Some("owner-only" | "anyone") => Ok(value),
        Some(_) => Err(ErrorData::invalid_params(
            "respond_to must be owner-only or anyone; choose specific people in Buzz Desktop",
            None,
        )),
    }
}

async fn publish_request<T: Serialize>(payload: ManagementRequest<T>) -> Result<(), ErrorData> {
    let channel_id = match serde_json::to_value(&payload.request)
        .ok()
        .and_then(|value| {
            value
                .get("channelId")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        }) {
        Some(channel_id) => channel_id,
        None => {
            return Err(ErrorData::invalid_params("channel_id is required", None));
        }
    };
    let event = ObserverEvent {
        seq: 0,
        timestamp: chrono::Utc::now().to_rfc3339(),
        kind: OBSERVER_EVENT_KIND,
        agent_index: None,
        channel_id: Some(channel_id),
        session_id: None,
        turn_id: None,
        payload,
    };

    let private_key = std::env::var("BUZZ_PRIVATE_KEY")
        .map_err(|_| ErrorData::internal_error("Buzz identity is unavailable", None))?;
    let keys = nostr::Keys::parse(&private_key)
        .map_err(|_| ErrorData::internal_error("Buzz identity is invalid", None))?;
    let auth_tag = std::env::var("BUZZ_AUTH_TAG")
        .map_err(|_| ErrorData::internal_error("This agent has no owner attestation", None))?;
    let owner = nip_oa::verify_auth_tag(&auth_tag, &keys.public_key()).map_err(|_| {
        ErrorData::internal_error("This agent's owner attestation is invalid", None)
    })?;
    let relay_url = std::env::var("BUZZ_RELAY_URL")
        .map_err(|_| ErrorData::internal_error("Buzz relay is unavailable", None))?;

    let encrypted = encrypt_observer_payload(&keys, &owner, &event).map_err(|error| {
        ErrorData::internal_error(format!("Could not encrypt request: {error}"), None)
    })?;
    let event = build_agent_observer_frame(
        &owner.to_hex(),
        &keys.public_key().to_hex(),
        OBSERVER_FRAME_TELEMETRY,
        &encrypted,
    )
    .map_err(|error| ErrorData::internal_error(format!("Could not build request: {error}"), None))?
    .sign_with_keys(&keys)
    .map_err(|error| ErrorData::internal_error(format!("Could not sign request: {error}"), None))?;
    let ws_url = relay_url
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    let auth = buzz_sdk::nip_oa::parse_auth_tag(&auth_tag).ok();
    let result = publish_event(&ws_url, event, &keys, auth.as_ref(), 10)
        .await
        .map_err(|error| {
            ErrorData::internal_error(format!("Could not send request: {error}"), None)
        })?;
    if !result.accepted {
        return Err(ErrorData::internal_error(
            "Buzz did not accept the request",
            None,
        ));
    }
    Ok(())
}

pub async fn create(mut request: CreateAgentParams) -> Result<CallToolResult, ErrorData> {
    request.channel_id = trim_required(&request.channel_id, "channel_id", 128)?;
    request.display_name = trim_required(&request.display_name, "display_name", MAX_NAME_CHARS)?;
    request.system_prompt =
        trim_required(&request.system_prompt, "system_prompt", MAX_PROMPT_CHARS)?;
    request.rationale = trim_required(&request.rationale, "rationale", MAX_RATIONALE_CHARS)?;
    request.runtime = validate_optional(request.runtime, "runtime")?;
    request.provider = validate_optional(request.provider, "provider")?;
    request.model = validate_optional(request.model, "model")?;
    request.respond_to = validate_respond_to(request.respond_to)?;
    let request_id = uuid::Uuid::new_v4().to_string();
    publish_request(ManagementRequest {
        request_type: REQUEST_KIND,
        action: "create",
        request_id: request_id.clone(),
        request,
    })
    .await?;
    text_result(format!("Agent creation is ready for the owner's review (request {request_id}). Do not claim it was created until Buzz confirms it."))
}

pub async fn update(mut request: UpdateAgentParams) -> Result<CallToolResult, ErrorData> {
    request.channel_id = trim_required(&request.channel_id, "channel_id", 128)?;
    request.agent_name = trim_required(&request.agent_name, "agent_name", MAX_NAME_CHARS)?;
    request.rationale = trim_required(&request.rationale, "rationale", MAX_RATIONALE_CHARS)?;
    request.display_name = request
        .display_name
        .map(|value| trim_required(&value, "display_name", MAX_NAME_CHARS))
        .transpose()?;
    request.system_prompt = request
        .system_prompt
        .map(|value| trim_required(&value, "system_prompt", MAX_PROMPT_CHARS))
        .transpose()?;
    request.runtime = validate_optional(request.runtime, "runtime")?;
    request.provider = validate_optional(request.provider, "provider")?;
    request.model = validate_optional(request.model, "model")?;
    request.respond_to = validate_respond_to(request.respond_to)?;
    if request.display_name.is_none()
        && request.system_prompt.is_none()
        && request.runtime.is_none()
        && request.provider.is_none()
        && request.model.is_none()
        && request.respond_to.is_none()
    {
        return Err(ErrorData::invalid_params(
            "Include at least one field to update",
            None,
        ));
    }
    let request_id = uuid::Uuid::new_v4().to_string();
    publish_request(ManagementRequest {
        request_type: REQUEST_KIND,
        action: "update",
        request_id: request_id.clone(),
        request,
    })
    .await?;
    text_result(format!("Agent update is ready for the owner's review (request {request_id}). Do not claim it was updated until Buzz confirms it."))
}
