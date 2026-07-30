use crate::shim::Shim;
use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::Deserialize;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const SEND_TIMEOUT: Duration = Duration::from_secs(20);

pub struct MessagingState {
    _shim: Shim,
    buzz_path: std::path::PathBuf,
}

impl MessagingState {
    pub fn new(shim: Shim) -> Self {
        let buzz_path = shim.buzz_path.clone();
        Self {
            _shim: shim,
            buzz_path,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendMessageParams {
    /// Channel UUID from the current Buzz context.
    pub channel: String,
    /// Message body.
    pub content: String,
    /// Optional event ID to reply to.
    #[serde(default)]
    pub reply_to: Option<String>,
    /// Optional public keys to mention.
    #[serde(default)]
    pub mentions: Vec<String>,
}

pub fn send_message_args(p: &SendMessageParams) -> Vec<String> {
    let mut args = vec![
        "messages".to_string(),
        "send".to_string(),
        "--channel".to_string(),
        p.channel.clone(),
        "--content".to_string(),
        "-".to_string(),
    ];
    if let Some(reply_to) = p.reply_to.as_deref().filter(|value| !value.is_empty()) {
        args.push("--reply-to".to_string());
        args.push(reply_to.to_string());
    }
    for mention in p.mentions.iter().filter(|value| !value.is_empty()) {
        args.push("--mention".to_string());
        args.push(mention.clone());
    }
    args
}

pub async fn send_message(
    state: &Arc<MessagingState>,
    p: SendMessageParams,
) -> Result<CallToolResult, ErrorData> {
    if p.channel.trim().is_empty() {
        return Ok(CallToolResult::error(vec![Content::text(
            "channel must not be empty",
        )]));
    }
    if p.content.trim().is_empty() {
        return Ok(CallToolResult::error(vec![Content::text(
            "content must not be empty",
        )]));
    }

    let mut command = Command::new(&state.buzz_path);
    command
        .args(send_message_args(&p))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = match tokio::time::timeout(SEND_TIMEOUT, async {
        let mut child = command
            .spawn()
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| ErrorData::internal_error("failed to open buzz stdin", None))?;
        stdin
            .write_all(p.content.as_bytes())
            .await
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        drop(stdin);
        child
            .wait_with_output()
            .await
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))
    })
    .await
    {
        Ok(result) => result?,
        Err(_) => {
            return Ok(CallToolResult::error(vec![Content::text(
                "buzz messages send timed out after 20 seconds",
            )]));
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if output.status.success() {
        Ok(CallToolResult::success(vec![Content::text(stdout)]))
    } else {
        let detail = if stderr.is_empty() { stdout } else { stderr };
        Ok(CallToolResult::error(vec![Content::text(format!(
            "buzz messages send failed ({}): {detail}",
            output.status
        ))]))
    }
}
