//! Supervisor-only managed-agent lifecycle control.
//!
//! Requests are signed Nostr events written to Buzz Desktop's local app-data
//! inbox. Desktop verifies the event signature and the requester's managed-agent
//! record before invoking its existing pair-scoped start/stop functions.

use nostr::{EventBuilder, JsonUtil, Kind, Tag};
use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const CONTROL_KIND: u16 = 29_110;
const SUPERVISOR_NAME: &str = "Buzz Management";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAction {
    Start,
    Stop,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeControlParams {
    /// Start the target before assignment or stop it after its callback.
    pub action: RuntimeAction,
    /// Exact managed-agent public key (hex).
    pub target_pubkey: String,
    /// Exact Buzz community relay URL for the runtime pair.
    pub relay_url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeControlPayload<'a> {
    action: &'a str,
    target_pubkey: &'a str,
    relay_url: &'a str,
    requested_at: u64,
    nonce: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeControlReceipt {
    ok: bool,
    action: String,
    target_pubkey: String,
    #[serde(default)]
    error: Option<String>,
}

pub async fn run(params: RuntimeControlParams) -> Result<CallToolResult, ErrorData> {
    let display_name = std::env::var("BUZZ_ACP_DISPLAY_NAME").unwrap_or_default();
    if display_name.trim() != SUPERVISOR_NAME {
        return Ok(error_result(
            "agent_runtime_control is restricted to the Buzz Management supervisor",
        ));
    }

    let target_pubkey = params.target_pubkey.trim().to_ascii_lowercase();
    if target_pubkey.len() != 64 || !target_pubkey.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(error_result("target_pubkey must be a 64-character hex key"));
    }
    let relay_url = params.relay_url.trim();
    if !(relay_url.starts_with("ws://") || relay_url.starts_with("wss://")) {
        return Ok(error_result("relay_url must start with ws:// or wss://"));
    }

    let private_key = std::env::var("BUZZ_PRIVATE_KEY")
        .map_err(|_| ErrorData::internal_error("BUZZ_PRIVATE_KEY is unavailable", None))?;
    let keys = nostr::Keys::parse(&private_key).map_err(|error| {
        ErrorData::internal_error(format!("invalid managed identity: {error}"), None)
    })?;
    let action = match params.action {
        RuntimeAction::Start => "start",
        RuntimeAction::Stop => "stop",
    };
    let requested_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let payload = RuntimeControlPayload {
        action,
        target_pubkey: &target_pubkey,
        relay_url,
        requested_at,
        nonce: format!("{}-{}", std::process::id(), requested_at),
    };
    let content = serde_json::to_string(&payload)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    let target_tag = Tag::parse(["p", target_pubkey.as_str()])
        .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
    let event = EventBuilder::new(Kind::Custom(CONTROL_KIND), content)
        .tags([target_tag])
        .sign_with_keys(&keys)
        .map_err(|error| {
            ErrorData::internal_error(format!("sign control request: {error}"), None)
        })?;

    let control_root = control_root()?;
    let inbox = control_root.join("inbox");
    let receipts = control_root.join("receipts");
    std::fs::create_dir_all(&inbox)
        .and_then(|_| std::fs::create_dir_all(&receipts))
        .map_err(|error| {
            ErrorData::internal_error(format!("create runtime-control queue: {error}"), None)
        })?;
    let event_id = event.id.to_hex();
    let request_path = inbox.join(format!("{event_id}.json"));
    let receipt_path = receipts.join(format!("{event_id}.json"));
    let event_json = event.as_json();
    let mut temp = tempfile::NamedTempFile::new_in(&inbox)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    use std::io::Write as _;
    temp.write_all(event_json.as_bytes())
        .and_then(|_| temp.flush())
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    temp.persist_noclobber(&request_path).map_err(|error| {
        ErrorData::internal_error(format!("queue runtime request: {}", error.error), None)
    })?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        match std::fs::read(&receipt_path) {
            Ok(bytes) => {
                let receipt: RuntimeControlReceipt =
                    serde_json::from_slice(&bytes).map_err(|error| {
                        ErrorData::internal_error(format!("parse runtime receipt: {error}"), None)
                    })?;
                let _ = std::fs::remove_file(&receipt_path);
                let text = if receipt.ok {
                    format!(
                        "{} completed for managed agent {}",
                        receipt.action, receipt.target_pubkey
                    )
                } else {
                    format!(
                        "{} failed for managed agent {}: {}",
                        receipt.action,
                        receipt.target_pubkey,
                        receipt.error.unwrap_or_else(|| "unknown error".into())
                    )
                };
                return Ok(if receipt.ok {
                    CallToolResult::success(vec![Content::text(text)])
                } else {
                    error_result(&text)
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(ErrorData::internal_error(
                    format!("read runtime receipt: {error}"),
                    None,
                ));
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(error_result(
                "Buzz Desktop did not acknowledge the runtime request within 20 seconds",
            ));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn control_root() -> Result<PathBuf, ErrorData> {
    if let Some(root) = std::env::var_os("BUZZ_RUNTIME_CONTROL_DIR") {
        return Ok(PathBuf::from(root));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return Ok(PathBuf::from(appdata)
            .join("xyz.block.buzz.app")
            .join("agents")
            .join("runtime-control"));
    }
    Err(ErrorData::internal_error(
        "runtime-control directory is unavailable",
        None,
    ))
}

fn error_result(message: &str) -> CallToolResult {
    CallToolResult::error(vec![Content::text(message.to_owned())])
}
