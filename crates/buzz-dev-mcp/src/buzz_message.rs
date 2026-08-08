use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use zeroize::Zeroize;

const MAX_CONTENT_BYTES: usize = 64 * 1024;
const MAX_RESPONSE_BYTES: usize = 256 * 1024;
const REQUEST_TIMEOUT_SECONDS: u64 = 30;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendMessageParams {
    /// Buzz channel UUID supplied in the turn's Context section.
    pub channel: String,
    /// Message body to publish.
    pub content: String,
    /// Optional event id supplied by the turn Context for a threaded reply.
    #[serde(default)]
    pub reply_to: Option<String>,
    /// Optional member public keys (hex or npub) to notify with p-tags.
    #[serde(default)]
    pub mentions: Vec<String>,
}

#[derive(Clone)]
pub struct PublisherClient {
    endpoint: String,
    capability: String,
}

#[derive(Serialize)]
struct PublishRequest<'a> {
    capability: &'a str,
    channel: &'a str,
    content: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to: Option<&'a str>,
    mentions: &'a [String],
}

impl PublisherClient {
    pub fn from_env() -> Option<Self> {
        let endpoint = std::env::var("BUZZ_PUBLISHER_ENDPOINT").ok();
        let capability = std::env::var("BUZZ_PUBLISHER_CAPABILITY").ok();
        std::env::remove_var("BUZZ_PUBLISHER_ENDPOINT");
        std::env::remove_var("BUZZ_PUBLISHER_CAPABILITY");
        let (endpoint, capability) = (endpoint?, capability?);
        if endpoint.trim().is_empty() || capability.trim().is_empty() {
            return None;
        }
        Some(Self {
            endpoint,
            capability,
        })
    }

    async fn publish(&self, params: &SendMessageParams) -> Result<Value, String> {
        let request = PublishRequest {
            capability: &self.capability,
            channel: &params.channel,
            content: &params.content,
            reply_to: params.reply_to.as_deref(),
            mentions: &params.mentions,
        };
        let payload = serde_json::to_vec(&request)
            .map_err(|error| format!("publisher request serialization failed: {error}"))?;
        let operation = async {
            let mut stream = TcpStream::connect(&self.endpoint)
                .await
                .map_err(|error| format!("publisher capability is unavailable: {error}"))?;
            stream
                .write_all(&payload)
                .await
                .map_err(|error| format!("publisher request failed: {error}"))?;
            stream
                .shutdown()
                .await
                .map_err(|error| format!("publisher request shutdown failed: {error}"))?;
            let mut response = Vec::new();
            stream
                .take((MAX_RESPONSE_BYTES + 1) as u64)
                .read_to_end(&mut response)
                .await
                .map_err(|error| format!("publisher response failed: {error}"))?;
            if response.len() > MAX_RESPONSE_BYTES {
                return Err("publisher response exceeds size limit".to_string());
            }
            serde_json::from_slice(&response)
                .map_err(|error| format!("publisher returned invalid JSON: {error}"))
        };
        tokio::time::timeout(
            std::time::Duration::from_secs(REQUEST_TIMEOUT_SECONDS),
            operation,
        )
        .await
        .map_err(|_| "publisher request timed out".to_string())?
    }
}

impl Drop for PublisherClient {
    fn drop(&mut self) {
        self.capability.zeroize();
    }
}

pub async fn run(
    publisher: Option<&PublisherClient>,
    params: SendMessageParams,
) -> Result<CallToolResult, ErrorData> {
    if params.content.trim().is_empty() {
        return Err(ErrorData::invalid_params(
            "content must not be empty".to_string(),
            None,
        ));
    }
    if params.content.len() > MAX_CONTENT_BYTES {
        return Err(ErrorData::invalid_params(
            format!("content exceeds {MAX_CONTENT_BYTES} bytes"),
            None,
        ));
    }
    let publisher = publisher.ok_or_else(|| {
        ErrorData::internal_error(
            "typed Buzz publisher is unavailable for this session".to_string(),
            None,
        )
    })?;
    let response = publisher
        .publish(&params)
        .await
        .map_err(|error| ErrorData::internal_error(error, None))?;
    let text = serde_json::to_string(&response).map_err(|error| {
        ErrorData::internal_error(
            format!("publisher response serialization failed: {error}"),
            None,
        )
    })?;
    if response.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(CallToolResult::success(vec![Content::text(text)]))
    } else {
        Ok(CallToolResult::error(vec![Content::text(text)]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn typed_tool_sends_fields_and_returns_structured_result() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind publisher");
        let endpoint = listener
            .local_addr()
            .expect("publisher address")
            .to_string();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept publisher request");
            let mut request = Vec::new();
            stream
                .read_to_end(&mut request)
                .await
                .expect("read publisher request");
            let request: Value = serde_json::from_slice(&request).expect("request JSON");
            stream
                .write_all(br#"{"ok":true,"event_id":"abc","accepted":true}"#)
                .await
                .expect("write publisher response");
            request
        });
        let publisher = PublisherClient {
            endpoint,
            capability: "opaque-session-capability".to_string(),
        };
        let result = run(
            Some(&publisher),
            SendMessageParams {
                channel: "a6e0737c-4205-4bcc-9741-2aad800e613f".to_string(),
                content: "hello".to_string(),
                reply_to: Some("f".repeat(64)),
                mentions: vec!["a".repeat(64)],
            },
        )
        .await
        .expect("typed tool result");

        assert!(!result.is_error.unwrap_or(false));
        let request = server.await.expect("publisher server");
        assert_eq!(request["capability"], "opaque-session-capability");
        assert_eq!(request["content"], "hello");
        assert_eq!(request["reply_to"], "f".repeat(64));
        assert_eq!(request["mentions"][0], "a".repeat(64));
    }

    #[tokio::test]
    async fn typed_tool_fails_closed_without_capability() {
        let error = run(
            None,
            SendMessageParams {
                channel: "a6e0737c-4205-4bcc-9741-2aad800e613f".to_string(),
                content: "hello".to_string(),
                reply_to: None,
                mentions: vec![],
            },
        )
        .await
        .expect_err("missing publisher must fail");

        assert!(error.message.contains("publisher is unavailable"));
    }
}
