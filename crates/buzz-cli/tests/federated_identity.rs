//! Process-boundary POC acceptance: real CLI -> assumed adapter -> protected HTTP.
//! No process-global environment mutation, no corporate service dependency.
use axum::{
    extract::State,
    http::HeaderMap,
    routing::{get, post},
    Json, Router,
};
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use nostr::Keys;
use sha2::{Digest, Sha256};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

// Use the test executable as a process launcher for the real CLI entrypoint.
// Tauri stages sidecar stubs over target/debug/buzz when sharing a target dir;
// this hashed executable is not a sidecar destination and cannot be clobbered.
#[tokio::test]
async fn cli_process_entry() {
    let Ok(args) = std::env::var("BUZZ_TEST_CLI_ARGS") else {
        return;
    };
    let args: Vec<String> = serde_json::from_str(&args).unwrap();
    std::process::exit(buzz_cli::run_from_args(args).await);
}

#[derive(Clone)]
struct Fixture {
    key: String,
    rejected: Arc<AtomicBool>,
    queries: Arc<AtomicUsize>,
}
fn proof(headers: &HeaderMap) -> nostr::Event {
    let bytes = STANDARD
        .decode(
            headers["authorization"]
                .to_str()
                .unwrap()
                .strip_prefix("Nostr ")
                .unwrap(),
        )
        .unwrap();
    let event: nostr::Event = serde_json::from_slice(&bytes).unwrap();
    event.verify().unwrap();
    event
}
async fn issue(
    State(f): State<Fixture>,
    headers: HeaderMap,
    body: String,
) -> Result<Json<serde_json::Value>, axum::http::StatusCode> {
    assert_eq!(
        headers["x-bb-session-credential"],
        "fixture-agent-capability"
    );
    assert!(!headers.contains_key("nostr-federated-identity"));
    let event = proof(&headers);
    assert_eq!(event.pubkey.to_hex(), f.key);
    assert!(event
        .tags
        .iter()
        .any(|tag| tag.as_slice() == ["payload", &hex::encode(Sha256::digest(body.as_bytes()))]));
    if f.rejected.load(Ordering::SeqCst) {
        return Err(axum::http::StatusCode::UNAUTHORIZED);
    }
    let now = nostr::Timestamp::now().as_secs();
    let token = format!("{}.{}.fixture", URL_SAFE_NO_PAD.encode(br#"{"typ":"nip-fi+jwt","alg":"ES256"}"#),
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&serde_json::json!({"iss":"fixture","sub":"agent","aud":"buzz","nostr_pubkey":f.key,"iat":now,"exp":now+120})).unwrap()));
    Ok(Json(
        serde_json::json!({"assertion":token,"nostr_pubkey":f.key,"expires_at":now+100}),
    ))
}
async fn query(
    State(f): State<Fixture>,
    headers: HeaderMap,
    body: String,
) -> Json<serde_json::Value> {
    let event = proof(&headers);
    assert_eq!(event.pubkey.to_hex(), f.key);
    assert_eq!(
        headers.get_all("nostr-federated-identity").iter().count(),
        1
    );
    assert!(headers["nostr-federated-identity"]
        .to_str()
        .unwrap()
        .starts_with("Bearer "));
    assert!(event
        .tags
        .iter()
        .any(|tag| tag.as_slice() == ["payload", &hex::encode(Sha256::digest(body.as_bytes()))]));
    assert!(!body.contains("fixture-agent-capability"));
    assert!(!body.contains("Bearer "));
    f.queries.fetch_add(1, Ordering::SeqCst);
    Json(serde_json::json!([]))
}
async fn socket(
    State(f): State<Fixture>,
    headers: HeaderMap,
    ws: axum::extract::WebSocketUpgrade,
) -> impl axum::response::IntoResponse {
    assert_eq!(
        headers.get_all("nostr-federated-identity").iter().count(),
        1
    );
    ws.on_upgrade(move |mut socket| async move {
        socket
            .send(axum::extract::ws::Message::Text(
                r#"["AUTH","fixture-challenge"]"#.into(),
            ))
            .await
            .unwrap();
        for expected in ["AUTH", "EVENT"] {
            let message = socket.recv().await.unwrap().unwrap();
            let text = message.to_text().unwrap();
            assert!(!text.contains("Bearer "));
            let value: serde_json::Value = serde_json::from_str(text).unwrap();
            assert_eq!(value[0], expected);
            let event: nostr::Event = serde_json::from_value(value[1].clone()).unwrap();
            event.verify().unwrap();
            assert_eq!(event.pubkey.to_hex(), f.key);
            let reply = serde_json::json!(["OK", event.id.to_hex(), true, ""]);
            socket
                .send(axum::extract::ws::Message::Text(reply.to_string().into()))
                .await
                .unwrap();
        }
    })
}
#[tokio::test]
async fn cli_acquires_its_own_assertion_and_stops_before_query_after_revocation() {
    let keys = Keys::generate();
    let fixture = Fixture {
        key: keys.public_key().to_hex(),
        rejected: Arc::default(),
        queries: Arc::default(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/", get(socket))
        .route("/assertions", post(issue))
        .route("/query", post(query))
        .with_state(fixture.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let run = |args: &[&str]| {
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "cli_process_entry", "--nocapture"])
            .env(
                "BUZZ_TEST_CLI_ARGS",
                serde_json::to_string(
                    &std::iter::once("buzz")
                        .chain(args.iter().copied())
                        .collect::<Vec<_>>(),
                )
                .unwrap(),
            )
            .env("BUZZ_PRIVATE_KEY", keys.secret_key().to_secret_hex())
            .env("BUZZ_RELAY_URL", &base)
            .env("BUZZ_NIP_FI_ORIGINS", &base)
            .env("BUZZ_NIP_FI_ENDPOINT", format!("{base}/assertions"))
            .env("BUZZ_NIP_FI_CREDENTIAL", "fixture-agent-capability")
            .env_remove("BUZZ_AUTH_TAG")
            .env_remove("GIT_CONFIG_COUNT")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        command.output()
    };
    let result = run(&["users", "get"]).await.unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(fixture.queries.load(Ordering::SeqCst) > 0);
    let result = run(&["users", "set-presence", "--status", "online"])
        .await
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let git_url = format!("{base}/git/owner/repo");
    let result = run(&[
        "git",
        "config",
        "--get-urlmatch",
        "http.extraHeader",
        &git_url,
    ])
    .await
    .unwrap();
    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("Nostr-Federated-Identity: Bearer "));
    let result = run(&[
        "git",
        "config",
        "--get-urlmatch",
        "http.extraHeader",
        "https://unrelated.example/git/repo",
    ])
    .await
    .unwrap();
    assert!(!String::from_utf8_lossy(&result.stdout).contains("Bearer "));
    let before = fixture.queries.load(Ordering::SeqCst);
    fixture.rejected.store(true, Ordering::SeqCst);
    let result = run(&["users", "get"]).await.unwrap();
    assert!(!result.status.success());
    assert_eq!(fixture.queries.load(Ordering::SeqCst), before);
    assert!(!String::from_utf8_lossy(&result.stderr).contains("fixture-agent-capability"));
    server.abort();
}
