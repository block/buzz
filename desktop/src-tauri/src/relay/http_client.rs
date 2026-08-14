use reqwest::header::{HeaderValue, AUTHORIZATION};

use super::{classify_intercepted_response, classify_request_error};
use crate::app_state::AppHttpClient;

pub(super) fn intercepted_response_message(response: &reqwest::Response) -> Option<String> {
    let final_host = response.url().host_str().unwrap_or("");
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    classify_intercepted_response(final_host, content_type)
}

async fn execute_relay_request_with_retry<F, A>(
    clients: &AppHttpClient,
    request: reqwest::Request,
    fresh_client: F,
    refresh_auth: A,
) -> Result<reqwest::Response, String>
where
    F: FnOnce() -> reqwest::Client,
    A: FnOnce() -> Result<String, String>,
{
    let retry_request = request.try_clone();
    let (client, generation) = clients.versioned();
    let response = client
        .execute(request)
        .await
        .map_err(|error| classify_request_error(&error))?;

    if intercepted_response_message(&response).is_none() {
        return Ok(response);
    }

    let Some(mut retry_request) = retry_request else {
        return Ok(response);
    };

    // The first request may have reached the relay's ingress before it returned
    // HTML. Replaying its NIP-98 event would trip the relay's event-id replay
    // gate, so the retry must carry a newly signed Authorization header.
    let refreshed_auth = HeaderValue::from_str(&refresh_auth()?)
        .map_err(|error| format!("authorization header failed: {error}"))?;
    retry_request
        .headers_mut()
        .insert(AUTHORIZATION, refreshed_auth);

    let fresh_client = fresh_client();
    let retry_response = fresh_client
        .execute(retry_request)
        .await
        .map_err(|error| classify_request_error(&error))?;

    // A non-intercepted response proves the fresh pool reached the intended
    // HTTP service. Heal the shared client so later requests do not repeatedly
    // pay for an intercepted first attempt. The generation check makes the
    // first concurrent successful recovery win.
    if intercepted_response_message(&retry_response).is_none() {
        clients.replace_if_generation(generation, fresh_client);
    }

    Ok(retry_response)
}

pub(super) async fn send_relay_request<A>(
    clients: &AppHttpClient,
    request: reqwest::RequestBuilder,
    refresh_auth: A,
) -> Result<reqwest::Response, String>
where
    A: FnOnce() -> Result<String, String>,
{
    let request = request
        .build()
        .map_err(|error| classify_request_error(&error))?;
    execute_relay_request_with_retry(
        clients,
        request,
        crate::app_state::build_app_http_client,
        refresh_auth,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::execute_relay_request_with_retry;
    use crate::{
        app_state::AppHttpClient,
        relay::{build_nip98_auth_header_for_keys, parse_json_response},
    };
    use reqwest::{header::HeaderMap, Method};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    fn client_with_generation(generation: &'static str) -> reqwest::Client {
        let mut headers = HeaderMap::new();
        headers.insert("x-client-generation", generation.parse().unwrap());
        reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .unwrap()
    }

    fn request_header(request: &str, name: &str) -> Option<String> {
        request.lines().find_map(|line| {
            let (header_name, value) = line.split_once(':')?;
            header_name
                .eq_ignore_ascii_case(name)
                .then(|| value.trim().to_string())
        })
    }

    #[tokio::test]
    async fn intercepted_html_refreshes_auth_and_heals_shared_client() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut observed_headers = Vec::new();
            for (content_type, body) in [
                ("text/html", "<html>sign in</html>"),
                ("application/json", "[]"),
                ("application/json", "[]"),
            ] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0u8; 4096];
                let read = stream.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..read]);
                observed_headers.push((
                    request_header(&request, "authorization"),
                    request_header(&request, "x-client-generation"),
                ));
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
            observed_headers
        });

        let clients = AppHttpClient::new(client_with_generation("initial"));
        let url = format!("http://{addr}/query");
        let body = b"[]";
        let keys = nostr::Keys::generate();
        let initial_auth =
            build_nip98_auth_header_for_keys(&keys, &Method::POST, &url, body).unwrap();
        let request = clients
            .post(&url)
            .header("Authorization", initial_auth)
            .body(body.to_vec())
            .build()
            .unwrap();

        let response = execute_relay_request_with_retry(
            &clients,
            request,
            || client_with_generation("fresh"),
            || build_nip98_auth_header_for_keys(&keys, &Method::POST, &url, body),
        )
        .await
        .unwrap();
        let events: Vec<serde_json::Value> = parse_json_response(response).await.unwrap();
        assert!(events.is_empty());

        let healed_response = clients.current().get(&url).send().await.unwrap();
        let healed_events: Vec<serde_json::Value> =
            parse_json_response(healed_response).await.unwrap();
        assert!(healed_events.is_empty());

        let observed_headers = server.await.unwrap();
        let first_auth = observed_headers[0].0.as_deref().unwrap();
        let retry_auth = observed_headers[1].0.as_deref().unwrap();
        assert_ne!(first_auth, retry_auth);
        assert_eq!(observed_headers[0].1.as_deref(), Some("initial"));
        assert_eq!(observed_headers[1].1.as_deref(), Some("fresh"));
        assert_eq!(observed_headers[2].1.as_deref(), Some("fresh"));
    }
}
