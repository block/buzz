use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[test]
fn official_openai_models_url_rejects_compat_routing_state() {
    let env = BTreeMap::from([(
        "OPENAI_COMPAT_BASE_URL".to_string(),
        "https://arbitrary-compatible-host.example/v1".to_string(),
    )]);
    let error = openai_compatible_models_url_for_discovery(Some("openai"), &env).unwrap_err();
    assert!(error.contains("provider=openai-compat"), "{error}");

    let legacy_canonical = BTreeMap::from([(
        "OPENAI_COMPAT_BASE_URL".to_string(),
        "https://api.openai.com/v1/".to_string(),
    )]);
    assert_eq!(
        openai_compatible_models_url_for_discovery(Some("openai"), &legacy_canonical).unwrap(),
        "https://api.openai.com/v1/models"
    );
}

#[tokio::test]
async fn official_openai_discovery_does_not_accept_compat_key() {
    let provider = effective_discovery_provider(Some("openai"), None, &BTreeMap::new());
    let env = BTreeMap::from([
        ("OPENAI_API_KEY".to_string(), "   ".to_string()),
        (
            "OPENAI_COMPAT_API_KEY".to_string(),
            "must-not-cross-provider-boundary".to_string(),
        ),
    ]);

    let error = discover_openai_compatible_models(&reqwest::Client::new(), &provider, &env, None)
        .await
        .unwrap_err();

    assert!(error.contains("OPENAI_API_KEY"), "{error}");
    assert!(!error.contains("OPENAI_COMPAT_API_KEY"), "{error}");
}

#[tokio::test]
async fn openai_compat_discovery_omits_authorization_without_key() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let captured = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 4096];
        while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = socket.read(&mut buffer).await.unwrap();
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..count]);
        }
        let body = r#"{"data":[{"id":"llama3","created":1}]}"#;
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        String::from_utf8_lossy(&bytes).to_ascii_lowercase()
    });

    let provider = effective_discovery_provider(Some("openai-compat"), None, &BTreeMap::new());
    let env = BTreeMap::from([
        ("OPENAI_COMPAT_BASE_URL".to_string(), base_url),
        (
            "OPENAI_API_KEY".to_string(),
            "must-not-cross-provider-boundary".to_string(),
        ),
    ]);
    let result = discover_openai_compatible_models(&reqwest::Client::new(), &provider, &env, None)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(result.models[0].id, "llama3");
    let request = captured.await.unwrap();
    assert!(request.starts_with("get /v1/models "), "{request}");
    assert!(!request.contains("authorization:"), "{request}");
}

#[test]
fn optional_compat_key_allows_explicit_blank_to_shadow_process_env() {
    let key = "BUZZ_TEST_OPENAI_COMPAT_API_KEY_OVERRIDE";
    let prior = std::env::var_os(key);
    std::env::set_var(key, "process-secret");

    let env = BTreeMap::from([(key.to_string(), "   ".to_string())]);
    assert_eq!(env_or_process_override(&env, key).as_deref(), Some(""));

    match prior {
        Some(value) => std::env::set_var(key, value),
        None => std::env::remove_var(key),
    }
}
