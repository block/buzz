use super::{classify_intercepted_response, classify_request_error};

pub(super) fn intercepted_response_message(response: &reqwest::Response) -> Option<String> {
    let final_host = response.url().host_str().unwrap_or("");
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    classify_intercepted_response(final_host, content_type)
}

async fn execute_relay_request_with_retry<F>(
    client: &reqwest::Client,
    request: reqwest::Request,
    fresh_client: F,
) -> Result<reqwest::Response, String>
where
    F: FnOnce() -> reqwest::Client,
{
    let retry_request = request.try_clone();
    let response = client
        .execute(request)
        .await
        .map_err(|error| classify_request_error(&error))?;

    if intercepted_response_message(&response).is_none() {
        return Ok(response);
    }

    let Some(retry_request) = retry_request else {
        return Ok(response);
    };

    fresh_client()
        .execute(retry_request)
        .await
        .map_err(|error| classify_request_error(&error))
}

pub(super) async fn send_relay_request(
    client: &reqwest::Client,
    request: reqwest::RequestBuilder,
) -> Result<reqwest::Response, String> {
    let request = request
        .build()
        .map_err(|error| classify_request_error(&error))?;
    execute_relay_request_with_retry(client, request, crate::app_state::build_app_http_client).await
}

#[cfg(test)]
mod tests {
    use super::execute_relay_request_with_retry;
    use crate::relay::parse_json_response;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    #[tokio::test]
    async fn intercepted_html_is_retried_with_a_fresh_client() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for (content_type, body) in [
                ("text/html", "<html>sign in</html>"),
                ("application/json", "[]"),
            ] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });

        let client = reqwest::Client::new();
        let request = client.get(format!("http://{addr}/query")).build().unwrap();
        let fresh_client_builds = Arc::new(AtomicUsize::new(0));
        let build_count = Arc::clone(&fresh_client_builds);

        let response = execute_relay_request_with_retry(&client, request, move || {
            build_count.fetch_add(1, Ordering::SeqCst);
            reqwest::Client::new()
        })
        .await
        .unwrap();
        let events: Vec<serde_json::Value> = parse_json_response(response).await.unwrap();

        assert!(events.is_empty());
        assert_eq!(fresh_client_builds.load(Ordering::SeqCst), 1);
        server.await.unwrap();
    }
}
