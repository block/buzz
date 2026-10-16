use std::{path::PathBuf, process::Stdio, time::Duration};

use buzz_sdk::nip_oa;
use buzz_ws_client::{NostrWsConnection, RelayMessage};
use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::json;
use tokio::{process::Command, time::timeout};

#[tokio::test]
#[ignore = "requires an isolated dev relay and installed terminal dependencies"]
async fn terminal_discovers_directory_agents_invites_sends_and_receives() {
    let url = std::env::var("BUZZ_TEST_RELAY_URL").expect("isolated relay URL");
    assert!(
        url.starts_with("ws://127.0.0.1:"),
        "local disposable relay only"
    );
    let human = Keys::generate();
    let agents = [Keys::generate(), Keys::generate()];
    let mut owner = NostrWsConnection::connect_authenticated(&url, &human, None)
        .await
        .expect("owner authentication");
    let mut responder = None;
    for (agent, name) in agents.iter().zip(["Aurora", "Borealis"]) {
        let raw = nip_oa::compute_auth_tag(&human, &agent.public_key(), "").expect("attestation");
        let tag: Vec<String> = serde_json::from_str(&raw).expect("auth tag");
        let auth = Tag::parse(tag).expect("tag");
        let mut conn = NostrWsConnection::connect_authenticated(&url, agent, Some(&auth))
            .await
            .expect("agent authentication");
        let profile = EventBuilder::new(Kind::Metadata, json!({"name":name}).to_string())
            .tags([auth.clone()])
            .sign_with_keys(agent)
            .expect("profile");
        let ack = conn
            .send_event(profile)
            .await
            .expect("profile acknowledgement");
        assert!(ack.accepted, "{}", ack.message);
        let policy = EventBuilder::new(
            Kind::Custom(30177),
            json!({"name":name,"respond_to":"owner-only","parallelism":1}).to_string(),
        )
        .tags([Tag::parse(["d", &agent.public_key().to_hex()]).expect("coordinate")])
        .sign_with_keys(&human)
        .expect("policy");
        let ack = owner
            .send_event(policy)
            .await
            .expect("policy acknowledgement");
        assert!(ack.accepted, "{}", ack.message);
        if name == "Borealis" {
            responder = Some((conn, auth));
        }
    }
    let (mut conn, auth) = responder.expect("responder");
    let agent = agents[1].clone();
    conn.send_raw(
        &json!(["REQ","membership",{"kinds":[44100],"#p":[agent.public_key().to_hex()],"limit":0}]),
    )
    .await
    .expect("agent subscription");
    let human_pubkey = human.public_key();
    let reply = tokio::spawn(async move {
        loop {
            if let RelayMessage::Event { event, .. } = conn
                .next_event(Duration::from_secs(60))
                .await
                .expect("agent receive")
            {
                if event.kind.as_u16() == 44100 {
                    let channel = event
                        .tags
                        .iter()
                        .find(|tag| tag.as_slice()[0] == "h")
                        .expect("invitation channel")
                        .as_slice()[1]
                        .clone();
                    conn.send_raw(&json!(["REQ","probe",{"kinds":[9],"#h":[channel],"limit":100}]))
                        .await
                        .expect("invited channel subscription");
                    continue;
                }
                if event.pubkey != human_pubkey {
                    continue;
                }
                event.verify().expect("human signature");
                let channel = event
                    .tags
                    .iter()
                    .find(|tag| tag.as_slice()[0] == "h")
                    .expect("channel")
                    .as_slice()[1]
                    .clone();
                let response = EventBuilder::new(Kind::Custom(9), "Probe received by Borealis")
                    .tags([
                        Tag::parse(["h", &channel]).expect("channel"),
                        Tag::parse(["e", &event.id.to_hex(), "", "reply"]).expect("thread"),
                        Tag::parse(["p", &human_pubkey.to_hex()]).expect("recipient"),
                        auth,
                    ])
                    .sign_with_keys(&agent)
                    .expect("reply");
                let ack = conn
                    .send_event(response)
                    .await
                    .expect("reply acknowledgement");
                assert!(ack.accepted, "{}", ack.message);
                break;
            }
        }
    });
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = timeout(
        Duration::from_secs(75),
        Command::new("node")
            .arg(root.join("terminal/test/live-relay.ts"))
            .env("BUZZ_RELAY_URL", &url)
            .env("BUZZ_PRIVATE_KEY", human.secret_key().to_secret_hex())
            .env("BUZZ_TEST_AGENT", agents[1].public_key().to_hex())
            .env(
                "BUZZ_TERMINAL_HOST",
                env!("CARGO_BIN_EXE_buzz-terminal-host"),
            )
            .env_remove("BUZZ_AGENT_PUBKEY")
            .env_remove("BUZZ_RELAY_PUBKEY")
            .env_remove("BUZZ_AUTH_TAG")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("TUI deadline")
    .expect("TUI process");
    if !output.status.success() {
        reply.abort();
    }
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    timeout(Duration::from_secs(5), reply)
        .await
        .expect("reply deadline")
        .expect("agent task");
    println!("{}", String::from_utf8_lossy(&output.stdout));
}
