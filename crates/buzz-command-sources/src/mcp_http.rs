use std::{fmt, time::Duration};

use reqwest::{
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE},
    redirect::Policy,
    StatusCode,
};
use serde_json::{json, Value};
use url::Url;

use crate::oauth::{WorldMonitorOAuthError, WorldMonitorOAuthStore};

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
    oauth: Option<WorldMonitorOAuthStore>,
    sessioned: bool,
    client: reqwest::Client,
}

impl fmt::Debug for McpHttpClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpHttpClient")
            .field("endpoint", &self.endpoint)
            .field("oauth", &self.oauth.as_ref().map(|_| "configured"))
            .field("sessioned", &self.sessioned)
            .finish_non_exhaustive()
    }
}

impl McpHttpClient {
    pub fn world_monitor(
        endpoint: &str,
        oauth: WorldMonitorOAuthStore,
    ) -> Result<Self, McpHttpError> {
        let endpoint = Url::parse(endpoint).map_err(|_| McpHttpError::InvalidEndpoint)?;
        if endpoint.scheme() != "https"
            || endpoint.host_str() != Some(WORLD_MONITOR_HOST)
            || endpoint.path() != "/mcp"
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(McpHttpError::InvalidEndpoint);
        }
        Self::new_with_oauth(endpoint, Some(oauth), false)
    }

    pub fn new(endpoint: Url) -> Result<Self, McpHttpError> {
        Self::new_with_oauth(endpoint, None, true)
    }

    fn new_with_oauth(
        endpoint: Url,
        oauth: Option<WorldMonitorOAuthStore>,
        sessioned: bool,
    ) -> Result<Self, McpHttpError> {
        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| McpHttpError::InvalidEndpoint)?;
        Ok(Self {
            endpoint,
            oauth,
            sessioned,
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
        let bearer = match &self.oauth {
            Some(store) => Some(store.bearer_token().await.map_err(map_oauth_error)?),
            None => None,
        };
        if self.sessioned {
            return self
                .session_request(id, method, params, bearer.as_ref())
                .await;
        }
        self.send_request(id, method, params, bearer.as_ref(), None)
            .await
            .map(|(result, _)| result)
    }

    async fn session_request(
        &self,
        id: u64,
        method: &str,
        params: Value,
        bearer: Option<&zeroize::Zeroizing<String>>,
    ) -> Result<Value, McpHttpError> {
        let (_, session_id) = self
            .send_request(
                1,
                "initialize",
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "buzz", "version": env!("CARGO_PKG_VERSION")}
                }),
                bearer,
                None,
            )
            .await?;
        let Some(session_id) = session_id else {
            return self
                .send_request(id, method, params, bearer, None)
                .await
                .map(|(result, _)| result);
        };
        self.send_notification("notifications/initialized", bearer, &session_id)
            .await?;
        let result = self
            .send_request(id, method, params, bearer, Some(&session_id))
            .await
            .map(|(result, _)| result);
        self.close_session(bearer, &session_id).await;
        result
    }

    async fn send_request(
        &self,
        id: u64,
        method: &str,
        params: Value,
        bearer: Option<&zeroize::Zeroizing<String>>,
        session_id: Option<&str>,
    ) -> Result<(Value, Option<String>), McpHttpError> {
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
        if let Some(bearer) = &bearer {
            request = request.header(AUTHORIZATION, format!("Bearer {}", bearer.as_str()));
        }
        if let Some(session_id) = session_id {
            request = request.header("mcp-session-id", session_id);
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
        let response_session_id = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
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
        parse_envelope(envelope, id).map(|result| (result, response_session_id))
    }

    async fn send_notification(
        &self,
        method: &str,
        bearer: Option<&zeroize::Zeroizing<String>>,
        session_id: &str,
    ) -> Result<(), McpHttpError> {
        let mut request = self
            .client
            .post(self.endpoint.clone())
            .header(ACCEPT, "application/json, text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .header("mcp-session-id", session_id)
            .json(&json!({"jsonrpc": "2.0", "method": method, "params": {}}));
        if let Some(bearer) = bearer {
            request = request.header(AUTHORIZATION, format!("Bearer {}", bearer.as_str()));
        }
        let response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                McpHttpError::Timeout
            } else {
                McpHttpError::Unavailable
            }
        })?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(McpHttpError::Unavailable)
        }
    }

    async fn close_session(&self, bearer: Option<&zeroize::Zeroizing<String>>, session_id: &str) {
        let mut request = self
            .client
            .delete(self.endpoint.clone())
            .header("mcp-session-id", session_id);
        if let Some(bearer) = bearer {
            request = request.header(AUTHORIZATION, format!("Bearer {}", bearer.as_str()));
        }
        let _ = request.send().await;
    }
}

fn map_oauth_error(error: WorldMonitorOAuthError) -> McpHttpError {
    match error {
        WorldMonitorOAuthError::NotConnected | WorldMonitorOAuthError::Reauthorise => {
            McpHttpError::Unauthorized
        }
        WorldMonitorOAuthError::State | WorldMonitorOAuthError::Unavailable => {
            McpHttpError::Unavailable
        }
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
    use crate::oauth::{WorldMonitorOAuthCredentials, WorldMonitorOAuthStore};
    use axum::{
        extract::State,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::post,
        Json, Router,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tempfile::tempdir;

    async fn session_mcp_post(
        State(stage): State<Arc<AtomicUsize>>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> axum::response::Response {
        let method = body.get("method").and_then(Value::as_str);
        let has_session = headers
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            == Some("test-session");
        match (method, stage.load(Ordering::SeqCst), has_session) {
            (Some("initialize"), 0, false) => {
                stage.store(1, Ordering::SeqCst);
                (
                    StatusCode::OK,
                    [("mcp-session-id", "test-session")],
                    Json(json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "result": {
                            "protocolVersion": "2025-06-18",
                            "capabilities": {},
                            "serverInfo": {"name": "session-test", "version": "1"}
                        }
                    })),
                )
                    .into_response()
            }
            (Some("notifications/initialized"), 1, true) => {
                stage.store(2, Ordering::SeqCst);
                StatusCode::ACCEPTED.into_response()
            }
            (Some("tools/call"), 2, true) => {
                stage.store(3, Ordering::SeqCst);
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "result": {"content": [{"type": "text", "text": "projected"}]}
                }))
                .into_response()
            }
            _ => StatusCode::BAD_REQUEST.into_response(),
        }
    }

    async fn session_mcp_delete(
        State(stage): State<Arc<AtomicUsize>>,
        headers: HeaderMap,
    ) -> StatusCode {
        let has_session = headers
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            == Some("test-session");
        if stage.load(Ordering::SeqCst) == 3 && has_session {
            stage.store(4, Ordering::SeqCst);
            StatusCode::OK
        } else {
            StatusCode::BAD_REQUEST
        }
    }

    async fn stateless_mcp_post(Json(body): Json<Value>) -> axum::response::Response {
        match body.get("method").and_then(Value::as_str) {
            Some("initialize") => Json(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "serverInfo": {"name": "stateless-test", "version": "1"}
                }
            }))
            .into_response(),
            Some("tools/call") => Json(json!({
                "jsonrpc": "2.0",
                "id": 2,
                "result": {"content": [{"type": "text", "text": "retrieved"}]}
            }))
            .into_response(),
            _ => StatusCode::BAD_REQUEST.into_response(),
        }
    }

    #[test]
    fn endpoint_accepts_only_https_world_monitor_mcp() {
        let directory = tempdir().expect("directory");
        let store =
            WorldMonitorOAuthStore::new(directory.path().join("oauth.json")).expect("store");
        assert!(
            McpHttpClient::world_monitor("https://api.worldmonitor.app/mcp", store.clone()).is_ok()
        );
        for endpoint in [
            "http://api.worldmonitor.app/mcp",
            "https://api.worldmonitor.app/api/mcp",
            "https://example.com/mcp",
            "https://api.worldmonitor.app/mcp?key=secret",
        ] {
            assert!(matches!(
                McpHttpClient::world_monitor(endpoint, store.clone()),
                Err(McpHttpError::InvalidEndpoint)
            ));
        }
    }

    #[test]
    fn debug_never_contains_oauth_tokens() {
        let directory = tempdir().expect("directory");
        let store =
            WorldMonitorOAuthStore::new(directory.path().join("oauth.json")).expect("store");
        store
            .save(
                &WorldMonitorOAuthCredentials::from_exchange(
                    "client".to_string(),
                    "access-never-print".to_string(),
                    "refresh-never-print".to_string(),
                    3600,
                )
                .expect("credentials"),
            )
            .expect("save");
        let client = McpHttpClient::world_monitor("https://api.worldmonitor.app/mcp", store)
            .expect("valid client");
        let debug = format!("{client:?}");
        assert!(!debug.contains("access-never-print"));
        assert!(!debug.contains("refresh-never-print"));
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

    #[tokio::test]
    async fn local_client_completes_streamable_http_session_before_tool_call() {
        let stage = Arc::new(AtomicUsize::new(0));
        let router = Router::new()
            .route("/mcp", post(session_mcp_post).delete(session_mcp_delete))
            .with_state(Arc::clone(&stage));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind session test server");
        let address = listener.local_addr().expect("session test address");
        tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve session test server");
        });
        let endpoint = Url::parse(&format!("http://{address}/mcp")).expect("endpoint");
        let client = McpHttpClient::new(endpoint).expect("local MCP client");

        let result = client
            .call_tool(
                "record_projected_event",
                json!({"source_event_id": "event-1"}),
            )
            .await
            .expect("session-aware tool call");

        assert_eq!(
            result["content"][0]["text"],
            Value::String("projected".to_string())
        );
        assert_eq!(stage.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn local_client_supports_stateless_mcp_servers_without_session_header() {
        let router = Router::new().route("/mcp", post(stateless_mcp_post));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind stateless test server");
        let address = listener.local_addr().expect("stateless test address");
        tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve stateless test server");
        });
        let endpoint = Url::parse(&format!("http://{address}/mcp")).expect("endpoint");
        let client = McpHttpClient::new(endpoint).expect("local MCP client");

        let result = client
            .call_tool("search_knowledge_base", json!({"query": "command"}))
            .await
            .expect("stateless tool call");

        assert_eq!(
            result["content"][0]["text"],
            Value::String("retrieved".to_string())
        );
    }
}
