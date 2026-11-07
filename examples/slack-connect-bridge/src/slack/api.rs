use std::{collections::HashMap, time::Duration};

use anyhow::{bail, Context, Result};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};

const SLACK_API_ORIGIN: &str = "https://slack.com/api";
const MAX_API_ATTEMPTS: usize = 3;
const MAX_RETRY_AFTER_SECS: u64 = 60;
const MAX_SLACK_TEXT_CHARS: usize = 39_000;

pub(crate) struct SlackClient {
    http: reqwest::Client,
    bot_token: String,
    api_origin: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SlackIdentity {
    pub(crate) team_id: String,
    pub(crate) user_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SlackConversation {
    #[serde(default)]
    pub(crate) is_ext_shared: bool,
    #[serde(default)]
    pub(crate) is_private: bool,
    #[serde(default)]
    pub(crate) is_archived: bool,
    #[serde(default)]
    pub(crate) name: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SlackPostedMessage {
    pub(crate) ts: String,
}

#[derive(Deserialize)]
struct ApiEnvelope {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(flatten)]
    rest: HashMap<String, Value>,
}

impl SlackClient {
    pub(crate) fn new(bot_token: String) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent("buzz-slack-connect-bridge/0.1")
            .build()
            .context("failed to build Slack HTTP client")?;
        Ok(Self {
            http,
            bot_token,
            api_origin: SLACK_API_ORIGIN.to_owned(),
        })
    }

    pub(crate) async fn auth_test(&self) -> Result<SlackIdentity> {
        let value = self.call(Method::POST, "auth.test", None).await?;
        Ok(SlackIdentity {
            team_id: required_string(&value, "team_id", "auth.test")?,
            user_id: required_string(&value, "user_id", "auth.test")?,
        })
    }

    pub(crate) async fn conversation_info(&self, channel_id: &str) -> Result<SlackConversation> {
        let value = self
            .call(
                Method::POST,
                "conversations.info",
                Some(json!({ "channel": channel_id })),
            )
            .await?;
        serde_json::from_value(
            value
                .get("channel")
                .cloned()
                .context("Slack conversations.info response omitted channel")?,
        )
        .context("invalid channel in Slack conversations.info response")
    }

    pub(crate) async fn user_display_name(&self, user_id: &str) -> Result<String> {
        let value = self
            .call(Method::POST, "users.info", Some(json!({ "user": user_id })))
            .await?;
        let user = value
            .get("user")
            .context("Slack users.info response omitted user")?;
        let profile = user
            .get("profile")
            .context("Slack users.info response omitted profile")?;
        for field in ["display_name", "real_name"] {
            if let Some(name) = profile.get(field).and_then(Value::as_str) {
                if !name.trim().is_empty() {
                    return Ok(name.trim().to_owned());
                }
            }
        }
        Ok(user
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(user_id)
            .to_owned())
    }

    pub(crate) async fn post_message(
        &self,
        channel_id: &str,
        text: &str,
        thread_ts: Option<&str>,
        client_msg_id: &str,
    ) -> Result<SlackPostedMessage> {
        let mut payload = json!({
            "channel": channel_id,
            "text": truncate_slack_text(text),
            "client_msg_id": client_msg_id,
            "unfurl_links": false,
            "unfurl_media": false
        });
        if let Some(thread_ts) = thread_ts {
            payload["thread_ts"] = Value::String(thread_ts.to_owned());
        }
        let value = self
            .call(Method::POST, "chat.postMessage", Some(payload))
            .await?;
        Ok(SlackPostedMessage {
            ts: required_string(&value, "ts", "chat.postMessage")?,
        })
    }

    async fn call(&self, method: Method, endpoint: &str, payload: Option<Value>) -> Result<Value> {
        let url = format!("{}/{endpoint}", self.api_origin);
        let mut last_error = None;

        for attempt in 0..MAX_API_ATTEMPTS {
            let mut request = self
                .http
                .request(method.clone(), &url)
                .bearer_auth(&self.bot_token);
            if let Some(payload) = &payload {
                request = request.json(payload);
            }

            let response = match request.send().await {
                Ok(response) => response,
                Err(error) => {
                    last_error = Some(
                        anyhow::Error::new(error)
                            .context(format!("Slack {endpoint} request failed")),
                    );
                    retry_transport(attempt).await;
                    continue;
                }
            };

            if response.status() == StatusCode::TOO_MANY_REQUESTS {
                let retry_after = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(1)
                    .min(MAX_RETRY_AFTER_SECS);
                last_error = Some(anyhow::anyhow!("Slack {endpoint} rate limited the bridge"));
                tokio::time::sleep(Duration::from_secs(retry_after)).await;
                continue;
            }

            let status = response.status();
            let body: Value = response
                .json()
                .await
                .with_context(|| format!("Slack {endpoint} returned non-JSON HTTP {status}"))?;
            if !status.is_success() {
                last_error = Some(anyhow::anyhow!("Slack {endpoint} returned HTTP {status}"));
                if status.is_server_error() {
                    retry_transport(attempt).await;
                    continue;
                }
                break;
            }

            let envelope: ApiEnvelope = serde_json::from_value(body)
                .with_context(|| format!("invalid Slack {endpoint} response"))?;
            if !envelope.ok {
                let code = envelope.error.unwrap_or_else(|| "unknown_error".to_owned());
                bail!("Slack {endpoint} failed: {code}");
            }
            return Ok(Value::Object(envelope.rest.into_iter().collect()));
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Slack {endpoint} request failed")))
    }
}

fn required_string(value: &Value, field: &str, endpoint: &str) -> Result<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .with_context(|| format!("Slack {endpoint} response omitted {field}"))
}

async fn retry_transport(attempt: usize) {
    if attempt + 1 < MAX_API_ATTEMPTS {
        let delay_ms = 250_u64.saturating_mul(1_u64 << attempt.min(4));
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
}

fn truncate_slack_text(text: &str) -> String {
    if text.chars().count() <= MAX_SLACK_TEXT_CHARS {
        return text.to_owned();
    }
    let mut truncated: String = text.chars().take(MAX_SLACK_TEXT_CHARS - 16).collect();
    truncated.push_str("\n… _(truncated)_");
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_preserves_utf8_boundaries() {
        let text = "🐝".repeat(MAX_SLACK_TEXT_CHARS + 10);
        let truncated = truncate_slack_text(&text);
        assert!(truncated.is_char_boundary(truncated.len()));
        assert!(truncated.chars().count() <= MAX_SLACK_TEXT_CHARS);
        assert!(truncated.ends_with("… _(truncated)_"));
    }
}
