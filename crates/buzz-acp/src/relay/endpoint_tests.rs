use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

const QUERY: &str = "token=a%2fb+%20&token=second&url=wss://other.example/&next=/";

#[tokio::test]
async fn endpoint_requests_preserve_path_query_and_signed_url() {
    for base_path in ["", "/", "/nostr", "/nostr/"] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        let server = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                // Read a complete request, including its body, before closing the socket.
                let request = tokio::time::timeout(Duration::from_secs(5), async {
                    let mut bytes = Vec::new();
                    loop {
                        let mut chunk = [0; 4096];
                        let n = socket.read(&mut chunk).await.unwrap();
                        assert!(n > 0, "incomplete request");
                        bytes.extend_from_slice(&chunk[..n]);
                        assert!(bytes.len() <= 32768, "test request too large");
                        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                            let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
                            let content_length = headers
                                .lines()
                                .filter_map(|line| line.split_once(':'))
                                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                                .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                                .unwrap_or(0);
                            if bytes.len() >= end + 4 + content_length {
                                break headers;
                            }
                        }
                    }
                })
                .await
                .unwrap();
                let first_line = request.lines().next().unwrap();
                let target = first_line.split_whitespace().nth(1).unwrap();
                let path = target.split('?').next().unwrap();
                let (status, body) = if first_line.starts_with("GET ") {
                    if path.ends_with("/info") {
                        (
                            "200 OK",
                            serde_json::json!({"self": "ab".repeat(32)}).to_string(),
                        )
                    } else {
                        ("404 Not Found", "{}".to_owned())
                    }
                } else {
                    ("200 OK", "[]".to_owned())
                };
                tx.send(request).await.unwrap();
                let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let client = RestClient {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            base_url: relay_ws_to_http(&format!("ws://{address}{base_path}?{QUERY}")),
            keys: Keys::generate(),
            auth_tag_json: None,
        };
        let filters = [nostr::Filter::new().kind(Kind::TextNote)];
        client.query(&filters).await.unwrap();
        client
            .query_raw(&[serde_json::json!({"kinds": [1]})])
            .await
            .unwrap();
        client.count(&filters).await.unwrap();
        let event = EventBuilder::new(Kind::TextNote, "test")
            .sign_with_keys(&client.keys)
            .unwrap();
        client.submit_event(&event).await.unwrap();
        assert_eq!(client.relay_self().await.unwrap(), Some("ab".repeat(32)));

        for (method, route) in [
            ("POST", "/query"),
            ("POST", "/query"),
            ("POST", "/count"),
            ("POST", "/events"),
            ("GET", "/"),
            ("GET", "/info"),
        ] {
            let request = tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .unwrap()
                .unwrap();
            let target = request
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let (path, query) = target.split_once('?').unwrap();
            assert!(request.starts_with(&format!("{method} ")));
            assert_eq!(path, format!("{}{route}", base_path.trim_end_matches('/')));
            assert_eq!(query, QUERY);
            if method == "POST" {
                let auth = request
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                    .unwrap()
                    .1
                    .trim()
                    .strip_prefix("Nostr ")
                    .unwrap();
                use base64::Engine;
                let event: nostr::Event = serde_json::from_slice(
                    &base64::engine::general_purpose::STANDARD
                        .decode(auth)
                        .unwrap(),
                )
                .unwrap();
                event.verify().unwrap();
                let signed_url = event.tags.iter().find(|t| t.as_slice()[0] == "u").unwrap();
                assert_eq!(
                    signed_url.as_slice()[1],
                    format!("http://{address}{target}")
                );
            } else {
                assert!(request
                    .lines()
                    .any(|line| line.eq_ignore_ascii_case("accept: application/nostr+json")));
            }
        }
        assert!(rx.try_recv().is_err(), "unexpected extra request");
        server.abort();
    }
}
