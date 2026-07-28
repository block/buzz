use std::{fmt, time::Duration};

use reqwest::{
    header::{ACCEPT, CONTENT_TYPE},
    redirect::Policy,
    StatusCode,
};
use serde_json::{json, Value};
use url::Url;
use zeroize::Zeroizing;

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const WORLD_MONITOR_HOST: &str = "api.worldmonitor.app";

#[derive(Debug, thiserror::Error)]
pub enum McpHttpError {
    #[error("invalid MCP endpoint")]
    InvalidEndpoint,
    #[error("MCP authentication failed")]
    Unauthorized,
    #[error("MCP quota is limited")]
    RateLimited,
    #[error("MCP request timed out")]
    Timeout,
    #[error("MCP redirect was rejected")]
    Redirect,
    #[error("MCP response was too large")]
    Oversized,
    #[error("MCP returned an error")]
    Mcp,
    #[error("MCP response was invalid")]
    InvalidResponse,
    #[error("MCP service was unavailable")]
    Unavailable,
}

pub struct McpHttpClient {
    endpoint: Url,
    api_key: Option<Zeroizing<String>>,
    client: reqwest::Client,
}

impl fmt::Debug for McpHttpClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpHttpClient")
            .field("endpoint", &self.endpoint)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .finish_non_exhaustive()
    }
}

impl McpHttpClient {
    pub fn world_monitor(endpoint: &str, api_key: String) -> Result<Self, McpHttpError> {
        let endpoint = Url::parse(endpoint).map_err(|_| McpHttpError::InvalidEndpoint)?;
        if endpoint.scheme() != "https"
            || endpoint.host_str() != Some(WORLD_MONITOR_HOST)
            || endpoint.path() != "/mcp"
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(McpHttpError::InvalidEndpoint);
        }
        Self::new(endpoint, Some(api_key))
    }

    pub fn new(endpoint: Url, api_key: Option<String>) -> Result<Self, McpHttpError> {
        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| McpHttpError::InvalidEndpoint)?;
        Ok(Self {
            endpoint,
            api_key: api_key.map(Zeroizing::new),
            client,
        })
    }

    pub async fn list_tools(&self) -> Result<Value, McpHttpError> {
        self.request(1, "tools/list", json!({})).await
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, McpHttpError> {
        self.request(
            2,
            "tools/call",
            json!({"name": name, "arguments": arguments}),
        )
        .await
    }

    async fn request(&self, id: u64, method: &str, params: Value) -> Result<Value, McpHttpError> {
        let mut request = self
            .client
            .post(self.endpoint.clone())
            .header(ACCEPT, "application/json, text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .json(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params
            }));
        if let Some(api_key) = &self.api_key {
            request = request.header("X-WorldMonitor-Key", api_key.as_str());
        }
        let response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                McpHttpError::Timeout
            } else {
                McpHttpError::Unavailable
            }
        })?;
        if response.status().is_redirection() {
            return Err(McpHttpError::Redirect);
        }
        match response.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(McpHttpError::Unauthorized);
            }
            StatusCode::TOO_MANY_REQUESTS => return Err(McpHttpError::RateLimited),
            status if !status.is_success() => return Err(McpHttpError::Unavailable),
            _ => {}
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err(McpHttpError::Oversized);
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .to_string();
        let bytes = response
            .bytes()
            .await
            .map_err(|_| McpHttpError::InvalidResponse)?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(McpHttpError::Oversized);
        }
        let envelope = if content_type == "text/event-stream" {
            parse_sse(&bytes)?
        } else if content_type == "application/json" || content_type.is_empty() {
            serde_json::from_slice(&bytes).map_err(|_| McpHttpError::InvalidResponse)?
        } else {
            return Err(McpHttpError::InvalidResponse);
        };
        parse_envelope(envelope, id)
    }
}

fn parse_sse(bytes: &[u8]) -> Result<Value, McpHttpError> {
    let text = std::str::from_utf8(bytes).map_err(|_| McpHttpError::InvalidResponse)?;
    text.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .filter(|data| !data.is_empty() && *data != "[DONE]")
        .filter_map(|data| serde_json::from_str::<Value>(data).ok())
        .next_back()
        .ok_or(McpHttpError::InvalidResponse)
}

fn parse_envelope(envelope: Value, id: u64) -> Result<Value, McpHttpError> {
    let object = envelope.as_object().ok_or(McpHttpError::InvalidResponse)?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || object.get("id").and_then(Value::as_u64) != Some(id)
    {
        return Err(McpHttpError::InvalidResponse);
    }
    if object.contains_key("error") {
        return Err(McpHttpError::Mcp);
    }
    let result = object
        .get("result")
        .cloned()
        .ok_or(McpHttpError::InvalidResponse)?;
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err(McpHttpError::Mcp);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_accepts_only_https_world_monitor_mcp() {
        assert!(McpHttpClient::world_monitor(
            "https://api.worldmonitor.app/mcp",
            "wm_live_redacted_value".to_string()
        )
        .is_ok());
        for endpoint in [
            "http://api.worldmonitor.app/mcp",
            "https://api.worldmonitor.app/api/mcp",
            "https://example.com/mcp",
            "https://api.worldmonitor.app/mcp?key=secret",
        ] {
            assert!(matches!(
                McpHttpClient::world_monitor(endpoint, "wm_live_redacted_value".to_string()),
                Err(McpHttpError::InvalidEndpoint)
            ));
        }
    }

    #[test]
    fn debug_never_contains_the_key() {
        let key = "wm_live_never_print_this";
        let client =
            McpHttpClient::world_monitor("https://api.worldmonitor.app/mcp", key.to_string())
                .expect("valid client");
        assert!(!format!("{client:?}").contains(key));
    }

    #[test]
    fn parses_json_and_sse_envelopes_strictly() {
        let json = json!({"jsonrpc":"2.0","id":2,"result":{"content":[]}});
        assert!(parse_envelope(json.clone(), 2).is_ok());
        assert!(parse_envelope(json, 1).is_err());
        let sse =
            b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"content\":[]}}\n\n";
        assert!(parse_envelope(parse_sse(sse).expect("SSE"), 2).is_ok());
        assert!(
            parse_envelope(json!({"jsonrpc":"2.0","id":2,"result":{"isError":true}}), 2).is_err()
        );
    }
}
