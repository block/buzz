use super::*;

const CONNECT_RETRY_DELAYS: [std::time::Duration; 3] = [
    std::time::Duration::from_millis(150),
    std::time::Duration::from_millis(350),
    std::time::Duration::from_millis(750),
];

/// Response from `POST /events`.
#[derive(Debug, Deserialize, serde::Serialize)]
pub struct SubmitEventResponse {
    pub event_id: String,
    pub accepted: bool,
    pub message: String,
}

async fn send_signed_event_request(
    client: &reqwest::Client,
    url: &str,
    auth_header: &str,
    body_bytes: &[u8],
) -> Result<reqwest::Response, String> {
    let mut retry_index = 0;
    loop {
        match client
            .post(url)
            .header("Authorization", auth_header)
            .header("Content-Type", "application/json")
            .body(body_bytes.to_vec())
            .send()
            .await
        {
            Ok(response) => return Ok(response),
            Err(error) if error.is_connect() && retry_index < CONNECT_RETRY_DELAYS.len() => {
                tokio::time::sleep(CONNECT_RETRY_DELAYS[retry_index]).await;
                retry_index += 1;
            }
            Err(error) => return Err(classify_request_error(&error)),
        }
    }
}

/// POST an already-signed event to an explicit relay with an explicit owner.
///
/// Deferred/scoped publication uses this form so a workspace or identity
/// switch cannot retarget either the event or its NIP-98 authentication after
/// the operation captured its `(relay, owner)` scope.
pub async fn submit_signed_event_at_with_keys(
    event: &nostr::Event,
    state: &AppState,
    api_base_url: &str,
    keys: &nostr::Keys,
) -> Result<SubmitEventResponse, String> {
    if event.pubkey != keys.public_key() {
        return Err("signed event does not match the publishing identity".to_string());
    }
    crate::relay_admission::wait_for_rate_limit().await;
    let url = format!("{}/events", api_base_url.trim_end_matches('/'));
    let body_bytes = event.as_json().into_bytes();
    crate::egress_guard::assert_no_key_backup_bytes(&body_bytes, "relay event submit")?;
    let auth_header = build_nip98_auth_header_for_keys(keys, &Method::POST, &url, &body_bytes)?;

    // Local proxies and stale pooled connections can reject a request before
    // any bytes reach the relay. Retry those connect failures with the exact
    // same signed event so a single click is reliable and remains idempotent.
    let client = if super::relay_url_bypasses_proxy(&url) {
        &state.direct_relay_http_client
    } else {
        &state.http_client
    };
    let response = send_signed_event_request(client, &url, &auth_header, &body_bytes).await?;

    if !response.status().is_success() {
        return Err(relay_error_message(response).await);
    }

    let result: SubmitEventResponse = parse_json_response(response).await?;
    if !result.accepted {
        return Err(format!("relay rejected event: {}", result.message));
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    #[tokio::test]
    async fn retries_connect_failure_with_the_same_request_body() {
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = probe.local_addr().unwrap();
        drop(probe);

        let server = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let listener = tokio::net::TcpListener::bind(address).await.unwrap();
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let read = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.contains("test-event"));
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                )
                .await
                .unwrap();
        });

        let response = send_signed_event_request(
            &reqwest::Client::new(),
            &format!("http://{address}/events"),
            "Nostr test-auth",
            b"test-event",
        )
        .await
        .unwrap();

        assert_eq!(response.status(), reqwest::StatusCode::OK);
        server.await.unwrap();
    }
}

/// Sign with an explicit identity and POST the event to an explicit relay.
///
/// The caller owns the signer lifetime. This is important for deferred work:
/// an in-process identity swap cannot retarget the event or its NIP-98 auth
/// after the caller has validated which identity the operation belongs to.
pub async fn submit_event_at_with_keys(
    builder: nostr::EventBuilder,
    state: &AppState,
    api_base_url: &str,
    keys: &nostr::Keys,
) -> Result<SubmitEventResponse, String> {
    let event = builder
        .sign_with_keys(keys)
        .map_err(|e| format!("failed to sign event: {e}"))?;
    submit_signed_event_at_with_keys(&event, state, api_base_url, keys).await
}

/// Build and submit an event to the currently active workspace relay.
pub async fn submit_event(
    builder: nostr::EventBuilder,
    state: &AppState,
) -> Result<SubmitEventResponse, String> {
    let keys = state.signing_keys()?;
    let event = builder
        .sign_with_keys(&keys)
        .map_err(|e| format!("failed to sign event: {e}"))?;
    let bases = super::relay_http_base_urls(state);
    let mut last_error = None;
    for (index, base) in bases.iter().enumerate() {
        match submit_signed_event_at_with_keys(&event, state, base, &keys).await {
            Ok(result) => return Ok(result),
            Err(error) if index + 1 < bases.len() && error.starts_with("relay unreachable:") => {
                last_error = Some(error);
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.unwrap_or_else(|| "relay unreachable: could not connect to relay".to_string()))
}
