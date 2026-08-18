use crate::shell::SharedState;
use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::Deserialize;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

const MAX_CONTENT_BYTES: usize = 64 * 1024;
const SEND_TIMEOUT: Duration = Duration::from_secs(30);
const CONTEXT_CHANNEL_ENV: &str = "BUZZ_CONTEXT_CHANNEL_ID";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BuzzMessageSendParams {
    /// Channel UUID from the current Buzz context.
    pub channel: String,
    /// Literal message body. It is passed as one argv value, never through a shell.
    pub content: String,
    #[serde(default)]
    pub kind: Option<u16>,
    #[serde(default)]
    pub reply_to: Option<String>,
    #[serde(default)]
    pub broadcast: bool,
    #[serde(default)]
    pub mentions: Vec<String>,
}

pub async fn run(
    state: &SharedState,
    p: BuzzMessageSendParams,
) -> Result<CallToolResult, ErrorData> {
    validate(&p)?;
    validate_context_channel(
        &p.channel,
        std::env::var(CONTEXT_CHANNEL_ENV).ok().as_deref(),
    )?;

    let mut command = Command::new(&state.shim.buzz_path);
    command
        .arg("messages")
        .arg("send")
        .arg("--channel")
        .arg(&p.channel)
        .arg("--content")
        .arg(&p.content)
        .current_dir(&state.cwd)
        .env("PATH", &state.shim.path_env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(kind) = p.kind {
        command.arg("--kind").arg(kind.to_string());
    }
    if let Some(reply_to) = &p.reply_to {
        command.arg("--reply-to").arg(reply_to);
    }
    if p.broadcast {
        command.arg("--broadcast");
    }
    for mention in &p.mentions {
        command.arg("--mention").arg(mention);
    }
    crate::configure_no_window_async(&mut command);

    let output = match tokio::time::timeout(SEND_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "failed to start fixed Buzz message sender: {error}"
            ))]));
        }
        Err(_) => {
            return Ok(CallToolResult::error(vec![Content::text(
                "Buzz message send timed out after 30 seconds",
            )]));
        }
    };

    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Ok(CallToolResult::error(vec![Content::text(format!(
            "Buzz message send failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            detail.trim()
        ))]));
    }
    let detail = String::from_utf8_lossy(&output.stdout);
    Ok(CallToolResult::success(vec![Content::text(
        if detail.trim().is_empty() {
            "Message sent".to_owned()
        } else {
            detail.trim().to_owned()
        },
    )]))
}

fn validate(p: &BuzzMessageSendParams) -> Result<(), ErrorData> {
    if p.channel.is_empty() || p.channel.len() > 128 || p.channel.starts_with('-') {
        return Err(ErrorData::invalid_params(
            "invalid channel identifier",
            None,
        ));
    }
    if p.content.is_empty() || p.content.len() > MAX_CONTENT_BYTES {
        return Err(ErrorData::invalid_params(
            format!("content must be 1..={MAX_CONTENT_BYTES} bytes"),
            None,
        ));
    }
    if p.reply_to
        .as_ref()
        .is_some_and(|value| value.is_empty() || value.len() > 128 || value.starts_with('-'))
    {
        return Err(ErrorData::invalid_params(
            "invalid reply_to identifier",
            None,
        ));
    }
    if p.mentions.len() > 32
        || p.mentions
            .iter()
            .any(|value| value.is_empty() || value.len() > 128 || value.starts_with('-'))
    {
        return Err(ErrorData::invalid_params("invalid mentions", None));
    }
    Ok(())
}

fn validate_context_channel(requested: &str, expected: Option<&str>) -> Result<(), ErrorData> {
    let Some(expected) = expected.filter(|value| !value.is_empty()) else {
        return Err(ErrorData::invalid_params(
            "Buzz message sending requires a host-bound channel context",
            None,
        ));
    };
    if requested != expected {
        return Err(ErrorData::invalid_params(
            "message channel does not match the host-bound conversation channel",
            None,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_flag_shaped_structured_values() {
        let params = BuzzMessageSendParams {
            channel: "--relay".into(),
            content: "hello".into(),
            kind: None,
            reply_to: None,
            broadcast: false,
            mentions: vec![],
        };
        assert!(validate(&params).is_err());
    }

    #[test]
    fn message_channel_is_bound_by_the_host() {
        assert!(validate_context_channel("channel-a", Some("channel-a")).is_ok());
        assert!(validate_context_channel("channel-b", Some("channel-a")).is_err());
        assert!(validate_context_channel("channel-a", None).is_err());
    }
}
