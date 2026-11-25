//! Runs the real `buzz` binary against a local fake relay to check that a
//! rebind reports the governed move as its one result, whether or not the
//! announcement republish after it succeeds.

use std::process::Command;
use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use axum::routing::post;
use axum::Router;
use tokio::net::TcpListener;

/// A relay that serves `owner`'s bound announcement and an absent bind
/// head, accepts governance commands, and answers announcements with
/// `announcement_accepted`. Returns its URL and the kinds submitted.
async fn relay(owner: nostr::Keys, announcement_accepted: bool) -> (String, Arc<Mutex<Vec<u64>>>) {
    let community = uuid::Uuid::new_v4();
    let announcement = nostr::EventBuilder::new(nostr::Kind::Custom(30617), "")
        .tags([
            nostr::Tag::parse(["d", "example"]).unwrap(),
            nostr::Tag::parse(["buzz-channel", &uuid::Uuid::new_v4().to_string()]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    let document = format!("bind:30617:{}:example", owner.public_key().to_hex());
    let submitted = Arc::new(Mutex::new(Vec::new()));
    let kinds = submitted.clone();
    let app = Router::new()
        .route(
            "/query",
            post(move |body: String| {
                let filters: serde_json::Value = serde_json::from_str(&body).unwrap();
                let reply = if filters[0].get("governance_head").is_some() {
                    serde_json::json!({"governance_head":1,"community_id":community,"document":document,"head":null})
                } else {
                    serde_json::json!([announcement])
                };
                async move { ([("content-type", "application/json")], reply.to_string()) }
            }),
        )
        .route(
            "/events",
            post(move |body: String| {
                let event: serde_json::Value = serde_json::from_str(&body).unwrap();
                let kind = event["kind"].as_u64().unwrap();
                kinds.lock().unwrap().push(kind);
                let accepted = kind != 30617 || announcement_accepted;
                let reply = serde_json::json!({"event_id":event["id"],"accepted":accepted,"message":if accepted {""} else {"invalid: stale"}});
                async move { (StatusCode::OK, [("content-type", "application/json")], reply.to_string()) }
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), submitted)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rebind_reports_the_move_once_and_warns_on_a_stale_announcement() {
    for announcement_accepted in [true, false] {
        let owner = nostr::Keys::generate();
        let (url, submitted) = relay(owner.clone(), announcement_accepted).await;
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_buzz"));
        cmd.env_clear()
            .env("BUZZ_PRIVATE_KEY", owner.secret_key().to_secret_hex())
            .args([
                "--relay",
                url.as_str(),
                "repos",
                "bind",
                "--id",
                "example",
                "--channel",
                &uuid::Uuid::new_v4().to_string(),
            ]);
        let output = tokio::task::spawn_blocking(move || cmd.output().unwrap())
            .await
            .unwrap();
        let (stdout, stderr) = (
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(
            output.status.success(),
            "accepted={announcement_accepted}: {stderr}"
        );
        let mut results =
            serde_json::Deserializer::from_str(&stdout).into_iter::<serde_json::Value>();
        let moved = results.next().expect("one result").unwrap();
        assert!(results.next().is_none(), "exactly one result: {stdout}");
        assert_eq!(moved["accepted"], true);
        assert_eq!(*submitted.lock().unwrap(), [9060, 30617]);
        assert_eq!(
            stderr.contains("warning:"),
            !announcement_accepted,
            "{stderr}"
        );
    }
}
