//! Public host policy discovery for the legacy desktop. No identity or sign-in trust.
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;
use url::Url;

const MAX_BYTES: usize = 256 * 1024;
const DEADLINE: Duration = Duration::from_secs(5);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayEntryRequirements {
    required: bool,
}

fn discovery_url(relay: &str) -> Result<Url, String> {
    let mut url = Url::parse(relay).map_err(|_| "Invalid community URL")?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
    {
        return Err("Community URL must not contain credentials, query or fragment".into());
    }
    let scheme = match url.scheme() {
        "ws" => "http",
        "wss" => "https",
        _ => return Err("Community URL must use ws or wss".into()),
    };
    url.set_scheme(scheme)
        .map_err(|_| "Invalid community scheme")?;
    Ok(url)
}

async fn fetch_policy(url: Url, deadline: Duration) -> Result<bool, String> {
    // This dedicated client has no cookies, default auth or redirect capability.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not initialize community discovery")?;
    tokio::time::timeout(deadline, async {
        let response = client
            .get(url)
            .header("Accept", "application/nostr+json")
            .header("Cache-Control", "no-cache")
            .send()
            .await
            .map_err(|_| "Could not reach community discovery")?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(format!(
                "Community discovery returned HTTP {}",
                response.status().as_u16()
            ));
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_BYTES as u64)
        {
            return Err("Community discovery response is too large".into());
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| "Could not read community discovery")?;
            if bytes.len() + chunk.len() > MAX_BYTES {
                return Err("Community discovery response is too large".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let doc: Value =
            serde_json::from_slice(&bytes).map_err(|_| "Invalid community discovery document")?;
        let object = doc
            .as_object()
            .ok_or("Invalid community discovery document")?;
        match object.get("requires_nip_fi") {
            None => Ok(false),
            Some(Value::Bool(required)) => Ok(*required),
            _ => Err("Invalid community authentication requirement".into()),
        }
    })
    .await
    .map_err(|_| "Community discovery timed out".to_string())?
}

#[tauri::command]
pub async fn fetch_relay_entry_requirements(
    relay_url: String,
) -> Result<RelayEntryRequirements, String> {
    let required = fetch_policy(discovery_url(&relay_url)?, DEADLINE).await?;
    Ok(RelayEntryRequirements { required })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    fn serve(status: &str, headers: &str, body: &str) -> (Url, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let response = format!("HTTP/1.1 {status}\r\nConnection: close\r\n{headers}\r\n{body}");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = vec![0; 4096];
            let size = socket.read(&mut request).unwrap();
            tx.send(String::from_utf8_lossy(&request[..size]).to_string())
                .ok();
            socket.write_all(response.as_bytes()).ok();
        });
        (Url::parse(&format!("http://{addr}/community")).unwrap(), rx)
    }

    #[tokio::test]
    async fn native_discovery_policy_and_network_boundaries() {
        for (body, expected) in [
            (r#"{}"#, Some(false)),
            (
                r#"{"requires_nip_fi":false,"limitation":{"federated_identity":true}}"#,
                Some(false),
            ),
            (
                r#"{"requires_nip_fi":true,"federated_identity":{"core":"unknown"}}"#,
                Some(true),
            ),
            (r#"{"requires_nip_fi":"true"}"#, None),
            (r#"{"requires_nip_fi":null}"#, None),
            ("[]", None),
            ("invalid", None),
            ("<!doctype html><html>Proxy sign-in</html>", None),
        ] {
            let (url, request) = serve("200 OK", "", body);
            let actual = fetch_policy(url.clone(), DEADLINE).await;
            assert_eq!(actual.ok(), expected, "{body}");
            let request = request.recv().unwrap().to_lowercase();
            assert!(request.starts_with("get /community http/1.1"));
            assert!(request.contains(&format!("host: {}", url.authority())));
            assert!(request.contains("accept: application/nostr+json"));
            assert!(!request.contains("authorization:"));
            assert!(!request.contains("cookie:"));
        }
        for status in [
            "404 Not Found",
            "503 Unavailable",
            "401 Unauthorized",
            "302 Found",
        ] {
            let (url, _) = serve(
                status,
                "Location: http://127.0.0.1:1/other\r\n",
                r#"{"requires_nip_fi":true}"#,
            );
            let error = fetch_policy(url, DEADLINE).await.unwrap_err();
            // A followed redirect would instead fail transport at port 1.
            assert!(error.contains(status.split(' ').next().unwrap()), "{error}");
        }
        let oversized = format!(
            "{{\"requires_nip_fi\":true,\"padding\":\"{}\"}}",
            "x".repeat(MAX_BYTES)
        );
        for headers in [
            String::new(),
            format!("Content-Length: {}\r\n", oversized.len()),
        ] {
            let (url, _) = serve("200 OK", &headers, &oversized);
            assert!(fetch_policy(url, DEADLINE)
                .await
                .unwrap_err()
                .contains("too large"));
        }
        // Both an unreachable endpoint and a body that never completes are errors.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        drop(listener);
        assert!(fetch_policy(url, DEADLINE).await.is_err());
        for partial in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
            let (release, hold) = mpsc::channel::<()>();
            std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                let mut request = [0; 4096];
                let size = socket.read(&mut request).unwrap();
                assert!(size > 0);
                if partial {
                    socket
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 99\r\n\r\n{")
                        .ok();
                }
                let _ = hold.recv_timeout(Duration::from_secs(10));
            });
            assert!(fetch_policy(url, Duration::from_millis(30))
                .await
                .unwrap_err()
                .contains("timed out"));
            drop(release);
        }
    }

    #[tokio::test]
    async fn pinned_producer_responses_use_the_native_reader() {
        for (body, required) in [
            (
                include_str!("../../tests/fixtures/nip_fi/buzz_main_6b8a72e835_required.json"),
                true,
            ),
            (
                include_str!("../../tests/fixtures/nip_fi/buzz_main_6b8a72e835_optional.json"),
                false,
            ),
            (
                include_str!(
                    "../../tests/fixtures/nip_fi/buzz_main_6b8a72e835_optional_capable.json"
                ),
                false,
            ),
        ] {
            let (url, _) = serve("200 OK", "", body);
            assert_eq!(fetch_policy(url, DEADLINE).await.unwrap(), required);
        }
    }

    #[test]
    fn arbitrary_community_urls_are_scoped() {
        for relay in [
            "wss://community.example",
            "ws://127.0.0.1:3000/tenant",
            "wss://[::1]:3443/",
        ] {
            assert!(discovery_url(relay).is_ok());
        }
        for relay in [
            "https://community.example",
            "file:///etc/passwd",
            "wss://user:secret@community.example",
            "wss://community.example?token=secret",
            "wss://community.example/#secret",
        ] {
            assert!(discovery_url(relay).is_err(), "{relay}");
        }
    }
}
