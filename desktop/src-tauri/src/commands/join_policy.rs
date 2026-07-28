use futures_util::StreamExt;
use serde_json::Value;
use std::time::Duration;
use url::Url;

// Each relay policy document is capped at 256 KiB before JSON encoding. Four
// MiB covers two maximally escaped documents plus the response envelope.
const MAX_JOIN_POLICY_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const JOIN_POLICY_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

fn join_policy_url(relay_url: &str) -> Result<Url, String> {
    let mut url = Url::parse(relay_url.trim()).map_err(|_| "invalid relay URL".to_string())?;
    let http_scheme = match url.scheme() {
        "wss" => "https",
        "ws" => "http",
        _ => return Err("relay URL must use ws:// or wss://".to_string()),
    };
    url.set_scheme(http_scheme)
        .map_err(|_| "invalid relay URL scheme".to_string())?;

    if !url.username().is_empty() || url.password().is_some() {
        return Err("relay URL must not contain credentials".to_string());
    }

    let base_path = url.path().trim_end_matches('/');
    url.set_path(&format!("{base_path}/api/join-policy"));
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

/// Fetch an arbitrary relay's optional join policy through native networking.
#[tauri::command]
pub async fn fetch_join_policy(relay_url: String) -> Result<Option<Value>, String> {
    let url = join_policy_url(&relay_url)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("failed to build join policy client: {error}"))?;
    let response = client
        .get(url)
        .timeout(JOIN_POLICY_REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|error| format!("join policy request failed: {error}"))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }

    let body = read_join_policy_json(response).await?;
    Ok(body
        .get("policy")
        .filter(|policy| !policy.is_null())
        .cloned())
}

async fn read_join_policy_json(response: reqwest::Response) -> Result<Value, String> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_JOIN_POLICY_RESPONSE_BYTES as u64)
    {
        return Err("relay returned oversized join policy".to_string());
    }

    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("reading join policy failed: {error}"))?;
        if bytes.len().saturating_add(chunk.len()) > MAX_JOIN_POLICY_RESPONSE_BYTES {
            return Err("relay returned oversized join policy".to_string());
        }
        bytes.extend_from_slice(&chunk);
    }

    serde_json::from_slice(&bytes).map_err(|_| "relay returned malformed join policy".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, Bytes},
        http::Response,
        response::Redirect,
        routing::get,
        Json, Router,
    };
    use std::convert::Infallible;

    async fn test_relay(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        format!("ws://{address}")
    }

    #[test]
    fn converts_relay_urls_to_join_policy_urls() {
        assert_eq!(
            join_policy_url("wss://relay.example.com/")
                .unwrap()
                .as_str(),
            "https://relay.example.com/api/join-policy"
        );
        assert_eq!(
            join_policy_url("ws://localhost:3000/base")
                .unwrap()
                .as_str(),
            "http://localhost:3000/base/api/join-policy"
        );
    }

    #[test]
    fn rejects_non_relay_schemes_and_credentials() {
        assert!(join_policy_url("https://relay.example.com").is_err());
        assert!(join_policy_url("wss://user:secret@relay.example.com").is_err());
    }

    #[tokio::test]
    async fn reads_an_optional_policy_without_webview_cors() {
        let relay_url = test_relay(Router::new().route(
            "/api/join-policy",
            get(|| async {
                Json(serde_json::json!({
                    "policy": {
                        "terms_markdown": "# Terms",
                        "age_attestation_required": true,
                        "version": "v1"
                    }
                }))
            }),
        ))
        .await;

        let policy = fetch_join_policy(relay_url).await.unwrap().unwrap();
        assert_eq!(policy["version"], "v1");
        assert_eq!(policy["age_attestation_required"], true);
    }

    #[tokio::test]
    async fn refuses_join_policy_redirects() {
        let relay_url = test_relay(Router::new().route(
            "/api/join-policy",
            get(|| async { Redirect::temporary("http://127.0.0.1:1/private") }),
        ))
        .await;

        assert_eq!(fetch_join_policy(relay_url).await.unwrap_err(), "HTTP 307");
    }

    #[tokio::test]
    async fn rejects_declared_oversized_join_policy() {
        let relay_url = test_relay(Router::new().route(
            "/api/join-policy",
            get(|| async {
                Response::builder()
                    .body(Body::from(vec![b'x'; MAX_JOIN_POLICY_RESPONSE_BYTES + 1]))
                    .unwrap()
            }),
        ))
        .await;

        assert_eq!(
            fetch_join_policy(relay_url).await.unwrap_err(),
            "relay returned oversized join policy"
        );
    }

    #[tokio::test]
    async fn rejects_chunked_oversized_join_policy() {
        let relay_url = test_relay(Router::new().route(
            "/api/join-policy",
            get(|| async {
                let chunk = Bytes::from(vec![b'x'; MAX_JOIN_POLICY_RESPONSE_BYTES + 1]);
                Body::from_stream(futures_util::stream::once(async move {
                    Ok::<_, Infallible>(chunk)
                }))
            }),
        ))
        .await;

        assert_eq!(
            fetch_join_policy(relay_url).await.unwrap_err(),
            "relay returned oversized join policy"
        );
    }
}
