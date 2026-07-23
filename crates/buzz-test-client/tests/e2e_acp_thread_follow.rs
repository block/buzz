//! End-to-end test for buzz-acp thread-following (#2270) against a live relay.
//!
//! Drives the real `buzz-acp` binary with a stub ACP agent (`stub-acp-agent`,
//! built from this crate) and asserts turn dispatch across the four cases from
//! the #2375 review:
//!
//! 1. explicit @mention → one turn (thread becomes followed);
//! 2. untagged human reply in the followed thread → one more turn;
//! 3. untagged agent-authored reply in the followed thread → no turn
//!    (followed-thread author policy, default `humans`);
//! 4. untagged top-level message → no turn (mention gate intact).
//!
//! Turn observation: the stub appends a line to `STUB_TURN_LOG` per
//! `session/prompt`. Participation is recorded by the harness at dispatch, so
//! turn counts alone prove admission/suppression — the stub never contacts the
//! relay (the agent's reply path is out-of-band by design; see #2459).
//!
//! Prerequisites:
//! - relay + Postgres running (`just setup`, `just relay`); `RELAY_URL`
//!   defaults to ws://localhost:3000
//! - `cargo build -p buzz-acp` (or set `BUZZ_ACP_BIN`)
//!
//! Run: `cargo test -p buzz-test-client --test e2e_acp_thread_follow -- --ignored`

use std::path::PathBuf;
use std::time::{Duration, Instant};

use nostr::{EventBuilder, Keys, Kind, Tag};

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn relay_http_url() -> String {
    relay_url()
        .replace("wss://", "https://")
        .replace("ws://", "http://")
        .trim_end_matches('/')
        .to_string()
}

/// Locate the buzz-acp binary: `BUZZ_ACP_BIN` override, else the workspace
/// target dir (release preferred, debug fallback).
fn buzz_acp_bin() -> PathBuf {
    if let Ok(path) = std::env::var("BUZZ_ACP_BIN") {
        return PathBuf::from(path);
    }
    let mut workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    workspace.pop();
    workspace.pop();
    for profile in ["release", "debug"] {
        let candidate = workspace.join("target").join(profile).join("buzz-acp");
        if candidate.exists() {
            return candidate;
        }
    }
    panic!("buzz-acp binary not found — run `cargo build -p buzz-acp` or set BUZZ_ACP_BIN");
}

/// POST a signed event to the relay's HTTP bridge.
async fn post_event(event: &nostr::Event) {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", event.pubkey.to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(event).expect("serialize event"))
        .send()
        .await
        .expect("submit event");
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.expect("parse event response");
    assert!(
        status.is_success() && body["accepted"].as_bool().unwrap_or(false),
        "event not accepted: {status} {body}"
    );
}

/// Create a channel via a signed kind:9007 event (creator becomes a member).
async fn create_test_channel(keys: &Keys) -> uuid::Uuid {
    let channel_uuid = uuid::Uuid::new_v4();
    let event = EventBuilder::new(Kind::Custom(9007), "")
        .tags(vec![
            Tag::parse(["h", &channel_uuid.to_string()]).unwrap(),
            Tag::parse(["name", &format!("acp-follow-e2e-{channel_uuid}")]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "open"]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();
    post_event(&event).await;
    channel_uuid
}

/// Add a member to a channel (kind:9030, signed by the channel owner).
async fn add_member(owner: &Keys, channel: uuid::Uuid, member: &Keys) {
    let event = buzz_sdk::build_add_member(channel, &member.public_key().to_hex(), None)
        .expect("build add-member")
        .sign_with_keys(owner)
        .expect("sign add-member");
    post_event(&event).await;
}

/// Publish a kind:9 channel message. `mention` adds a `p` tag; `thread_root`
/// adds a NIP-10 marker `e` tag.
async fn send_channel_message(
    keys: &Keys,
    channel: uuid::Uuid,
    content: &str,
    mention: Option<&Keys>,
    thread_root: Option<&str>,
) -> String {
    let mut tags = vec![Tag::parse(["h", &channel.to_string()]).unwrap()];
    if let Some(m) = mention {
        tags.push(Tag::parse(["p", &m.public_key().to_hex()]).unwrap());
    }
    if let Some(root) = thread_root {
        tags.push(Tag::parse(["e", root, "", "root"]).unwrap());
    }
    let event = EventBuilder::new(Kind::Custom(9), content)
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap();
    let id = event.id.to_hex();
    post_event(&event).await;
    id
}

fn turn_count(log: &std::path::Path) -> usize {
    std::fs::read_to_string(log)
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

/// Deadline-poll until `turn_count` reaches `expected` (same shape as the
/// scheduler-push waits in e2e_event_reminder.rs).
async fn await_turns(log: &std::path::Path, expected: usize, deadline: Duration) {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if turn_count(log) >= expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!(
        "expected {expected} dispatched turn(s) within {deadline:?}, saw {}",
        turn_count(log)
    );
}

/// Assert `turn_count` stays at `expected` for the whole `window`.
async fn assert_no_new_turns(log: &std::path::Path, expected: usize, window: Duration) {
    let start = Instant::now();
    while start.elapsed() < window {
        let n = turn_count(log);
        assert!(
            n <= expected,
            "unexpected turn dispatched: saw {n}, expected {expected}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Kills the harness on drop so a failing assertion can't leak the subprocess.
struct HarnessGuard(std::process::Child);
impl Drop for HarnessGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
#[ignore]
async fn thread_follow_admits_humans_and_suppresses_agents() {
    let human = Keys::generate();
    let agent = Keys::generate();
    let other_agent = Keys::generate();

    let channel = create_test_channel(&human).await;
    add_member(&human, channel, &agent).await;
    add_member(&human, channel, &other_agent).await;

    // Publish a kind:0 for the second agent carrying the NIP-OA `auth` tag
    // shape (4 elements) — the marker `profile_event_is_agent` classifies on.
    let profile = EventBuilder::new(Kind::Metadata, r#"{"name":"other-agent"}"#)
        .tags(vec![Tag::parse([
            "auth",
            &human.public_key().to_hex(),
            "",
            "e2e-attestation-placeholder",
        ])
        .unwrap()])
        .sign_with_keys(&other_agent)
        .unwrap();
    post_event(&profile).await;

    // Spawn the real harness with the stub ACP agent.
    let scratch = std::env::temp_dir().join(format!("acp-follow-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&scratch).expect("create scratch dir");
    let turn_log = scratch.join("turns.log");
    let harness_log = scratch.join("harness.log");
    let log_file = std::fs::File::create(&harness_log).expect("create harness log");

    let child = std::process::Command::new(buzz_acp_bin())
        .env("BUZZ_PRIVATE_KEY", agent.secret_key().to_secret_hex())
        .env("BUZZ_RELAY_URL", relay_url())
        .env(
            "BUZZ_ACP_AGENT_COMMAND",
            env!("CARGO_BIN_EXE_stub-acp-agent"),
        )
        .env("BUZZ_ACP_RESPOND_TO", "anyone")
        .env("BUZZ_ACP_HEARTBEAT_INTERVAL", "0")
        .env("BUZZ_ACP_AGENTS", "1")
        .env("STUB_TURN_LOG", &turn_log)
        .env("RUST_LOG", "buzz_acp=debug")
        .stdout(log_file.try_clone().expect("clone log handle"))
        .stderr(log_file)
        .spawn()
        .expect("spawn buzz-acp");
    let _guard = HarnessGuard(child);

    // Readiness: the harness logs its channel subscription.
    let subscribed = format!("subscribed to channel {channel}");
    let start = Instant::now();
    loop {
        let log = std::fs::read_to_string(&harness_log).unwrap_or_default();
        if log.contains(&subscribed) {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "harness did not subscribe to {channel} within 30s; log:\n{log}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    // 1. Explicit mention → one turn; the thread root (= mention id) becomes followed.
    let mention_id = send_channel_message(&human, channel, "@agent ping", Some(&agent), None).await;
    await_turns(&turn_log, 1, Duration::from_secs(30)).await;

    // 2. Untagged human reply in the followed thread → a second turn.
    send_channel_message(
        &human,
        channel,
        "untagged follow-up",
        None,
        Some(&mention_id),
    )
    .await;
    await_turns(&turn_log, 2, Duration::from_secs(30)).await;

    // 3. Untagged agent-authored reply in the followed thread → suppressed.
    //    respond-to=anyone admits the author, so the followed-thread author
    //    policy is the only gate exercised here.
    send_channel_message(
        &other_agent,
        channel,
        "untagged agent interjection",
        None,
        Some(&mention_id),
    )
    .await;
    assert_no_new_turns(&turn_log, 2, Duration::from_secs(10)).await;
    let log = std::fs::read_to_string(&harness_log).unwrap_or_default();
    assert!(
        log.contains("followed-thread admission suppressed for agent author"),
        "expected suppression log line; harness log:\n{log}"
    );

    // 4. Untagged top-level message → no turn (mention gate intact).
    send_channel_message(&human, channel, "no mention here", None, None).await;
    assert_no_new_turns(&turn_log, 2, Duration::from_secs(10)).await;
}
