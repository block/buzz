//! Client for the harness-owned constrained reply broker.

use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const RESPONSE_LIMIT: usize = 16 * 1024;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReplyParams {
    /// UUID of the current Buzz channel.
    pub channel_id: String,
    /// 64-character hexadecimal ID of the message being replied to.
    pub reply_to: String,
    /// Markdown message content. The broker enforces Buzz's 64 KiB limit.
    pub content: String,
    /// Stable key for retrying this exact reply. Omit on a first attempt; reuse
    /// the returned key only when the original outcome is unknown.
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Serialize)]
struct BrokerRequest {
    capability: String,
    channel_id: String,
    reply_to: String,
    content: String,
    idempotency_key: String,
}

#[derive(Deserialize)]
struct BrokerResponse {
    ok: bool,
    event_id: Option<String>,
    #[allow(dead_code)]
    state: Option<String>,
    error: Option<String>,
}

pub async fn send(params: ReplyParams) -> Result<String, ErrorData> {
    let endpoint = std::env::var("BUZZ_REPLY_BROKER_URL").map_err(|_| {
        ErrorData::internal_error(
            "relay reply broker is not configured; raw Buzz publishing is unavailable to workers",
            None,
        )
    })?;
    let capability = std::env::var("BUZZ_REPLY_BROKER_CAPABILITY").map_err(|_| {
        ErrorData::internal_error("relay reply broker capability is unavailable", None)
    })?;
    let address = endpoint
        .strip_prefix("tcp://")
        .ok_or_else(|| ErrorData::internal_error("relay reply broker endpoint is invalid", None))?;
    let idempotency_key = params
        .idempotency_key
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
    let request = BrokerRequest {
        capability,
        channel_id: params.channel_id,
        reply_to: params.reply_to,
        content: params.content,
        idempotency_key: idempotency_key.clone(),
    };
    let payload = serde_json::to_vec(&request).map_err(|error| {
        ErrorData::internal_error(format!("reply request encode failed: {error}"), None)
    })?;
    let mut stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(address))
        .await
        .map_err(|_| ErrorData::internal_error("relay reply broker connection timed out", None))?
        .map_err(|error| {
            ErrorData::internal_error(format!("relay reply broker unavailable: {error}"), None)
        })?;
    stream.write_all(&payload).await.map_err(|error| {
        ErrorData::internal_error(format!("relay reply broker write failed: {error}"), None)
    })?;
    stream.write_all(b"\n").await.map_err(|error| {
        ErrorData::internal_error(format!("relay reply broker write failed: {error}"), None)
    })?;
    let mut response = String::new();
    let mut reader = BufReader::new(stream);
    let bytes = tokio::time::timeout(CONNECT_TIMEOUT, reader.read_line(&mut response))
        .await
        .map_err(|_| ErrorData::internal_error("relay reply broker response timed out", None))?
        .map_err(|error| {
            ErrorData::internal_error(format!("relay reply broker read failed: {error}"), None)
        })?;
    if bytes == 0 || response.len() > RESPONSE_LIMIT {
        return Err(ErrorData::internal_error(
            "relay reply broker returned an invalid response",
            None,
        ));
    }
    let response: BrokerResponse = serde_json::from_str(&response).map_err(|error| {
        ErrorData::internal_error(
            format!("relay reply broker response decode failed: {error}"),
            None,
        )
    })?;
    if response.ok {
        let event_id = response.event_id.ok_or_else(|| {
            ErrorData::internal_error("relay reply broker omitted the event id", None)
        })?;
        Ok(format!(
            "Reply accepted. event_id={event_id}; idempotency_key={idempotency_key}"
        ))
    } else {
        let error = response
            .error
            .unwrap_or_else(|| "relay reply broker rejected the reply".into());
        Err(ErrorData::internal_error(
            format!("{error}; idempotency_key={idempotency_key}"),
            None,
        ))
    }
}
