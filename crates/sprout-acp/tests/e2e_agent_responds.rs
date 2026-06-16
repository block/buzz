//! End-to-end test: a serverless agent actually RESPONDS in a channel.
//!
//! This is the test that proves the whole chain works, against real public
//! relays, with no LLM:
//!
//!   1. create a channel (kind:39000) + add the agent as a member (kind:39002)
//!   2. spawn the real `sprout-acp` binary with a STUB agent (a shell script
//!      that speaks minimal ACP and posts a reply via the `sprout` CLI)
//!   3. publish a message mentioning the agent
//!   4. assert the agent's reply lands on the relay
//!
//! It exercises: serverless detection (comma relay list), multi-relay connect,
//! channel discovery (39002 over WS), subscription, the respond gate, the ACP
//! prompt turn, and the reply publish via the `sprout` CLI.
//!
//! Run with:
//!   cargo test -p sprout-acp --test e2e_agent_responds -- --ignored --nocapture

use std::process::Stdio;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nostr::{EventBuilder, Keys, Kind, ToBech32};
use tokio::process::Command;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const RELAYS_DEFAULT: &str = "wss://relay.damus.io,wss://nos.lol";

fn relays() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| RELAYS_DEFAULT.to_string())
}

/// Publish a signed event to one relay over plain WS and wait briefly for OK.
/// Tolerant: a relay hiccup (503, connect error, rejected) does not panic —
/// we publish to multiple relays and only need one to accept.
async fn publish(relay: &str, event: &nostr::Event) -> bool {
    let ws = match connect_async(relay).await {
        Ok((ws, _)) => ws,
        Err(e) => {
            eprintln!("  (publish connect to {relay} failed: {e} — skipping)");
            return false;
        }
    };
    let (mut write, mut read) = ws.split();
    let msg = serde_json::json!(["EVENT", event]).to_string();
    if write.send(Message::Text(msg.into())).await.is_err() {
        return false;
    }
    // Drain briefly for the OK; report whether it was accepted.
    let accepted = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(Ok(m)) = read.next().await {
            if let Message::Text(t) = m {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
                    let arr = v.as_array().cloned().unwrap_or_default();
                    if arr.first().and_then(|x| x.as_str()) == Some("OK") {
                        let ok = arr.get(2).and_then(|x| x.as_bool()).unwrap_or(false);
                        if !ok {
                            eprintln!("  (publish to {relay} rejected: {t})");
                        }
                        return ok;
                    }
                }
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    let _ = write.close().await;
    accepted
}

/// Publish to every relay in a comma list; returns true if any accepted.
async fn publish_all(relay_list: &str, event: &nostr::Event) -> bool {
    let mut any = false;
    for r in relay_list.split(',') {
        if publish(r.trim(), event).await {
            any = true;
        }
    }
    any
}

/// Query one relay for events matching a filter; collect until EOSE/timeout.
/// Tolerant: a relay hiccup (503, connect error) returns empty rather than
/// panicking — the caller retries across attempts/relays.
async fn query(relay: &str, filter: serde_json::Value) -> Vec<nostr::Event> {
    let ws = match connect_async(relay).await {
        Ok((ws, _)) => ws,
        Err(e) => {
            eprintln!("  (query connect to {relay} failed: {e} — treating as empty)");
            return Vec::new();
        }
    };
    let (mut write, mut read) = ws.split();
    let sub = "q1";
    let req = serde_json::json!(["REQ", sub, filter]).to_string();
    write.send(Message::Text(req.into())).await.expect("req");
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(8), async {
        while let Some(Ok(m)) = read.next().await {
            if let Message::Text(t) = m {
                let v: serde_json::Value = match serde_json::from_str(&t) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let arr = v.as_array().cloned().unwrap_or_default();
                match arr.first().and_then(|x| x.as_str()) {
                    Some("EVENT") if arr.get(1).and_then(|x| x.as_str()) == Some(sub) => {
                        if let Some(ev) = arr.get(2) {
                            if let Ok(e) = serde_json::from_value::<nostr::Event>(ev.clone()) {
                                out.push(e);
                            }
                        }
                    }
                    Some("EOSE") => break,
                    _ => {}
                }
            }
        }
    })
    .await;
    let _ = write.close().await;
    out
}

/// Open a LIVE subscription (no `until`, `since=now`) on one relay and watch for
/// an event matching `pred` for up to `secs`. Used for ephemeral events (typing
/// indicators, kind 20002) that relays do NOT store, so a normal `query` after
/// the fact returns nothing — they must be caught live as they stream.
async fn listen_live(
    relay: &str,
    filter: serde_json::Value,
    secs: u64,
    pred: impl Fn(&nostr::Event) -> bool,
) -> bool {
    let ws = match connect_async(relay).await {
        Ok((ws, _)) => ws,
        Err(e) => {
            eprintln!("  (listen connect to {relay} failed: {e})");
            return false;
        }
    };
    let (mut write, mut read) = ws.split();
    let sub = "live1";
    let req = serde_json::json!(["REQ", sub, filter]).to_string();
    if write.send(Message::Text(req.into())).await.is_err() {
        return false;
    }
    let found = tokio::time::timeout(Duration::from_secs(secs), async {
        while let Some(Ok(m)) = read.next().await {
            if let Message::Text(t) = m {
                let v: serde_json::Value = match serde_json::from_str(&t) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let arr = v.as_array().cloned().unwrap_or_default();
                if arr.first().and_then(|x| x.as_str()) == Some("EVENT")
                    && arr.get(1).and_then(|x| x.as_str()) == Some(sub)
                {
                    if let Some(ev) = arr.get(2) {
                        if let Ok(e) = serde_json::from_value::<nostr::Event>(ev.clone()) {
                            if pred(&e) {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    let _ = write.close().await;
    found
}

/// Poll all relays (merged) for a kind-9 reply from `agent_pk` whose content
/// contains `marker`. Returns true if found within `attempts` × 3s.
async fn await_reply(
    relay_list: &str,
    channel: &str,
    agent_pk: nostr::PublicKey,
    marker: &str,
    attempts: u32,
) -> bool {
    for attempt in 0..attempts {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let mut events: Vec<nostr::Event> = Vec::new();
        for r in relay_list.split(',') {
            let mut got = query(
                r.trim(),
                serde_json::json!({"kinds":[9],"#h":[channel],"limit":50}),
            )
            .await;
            events.append(&mut got);
        }
        if events
            .iter()
            .any(|e| e.content.contains(marker) && e.pubkey == agent_pk)
        {
            eprintln!("✅ reply '{marker}' found after {}s", (attempt + 1) * 3);
            return true;
        }
        eprintln!(
            "  …waiting for '{marker}': attempt {attempt}, {} msgs",
            events.len()
        );
    }
    false
}

#[tokio::test]
#[ignore = "network: hits live public relays; spawns sprout-acp + sprout binaries"]
async fn agent_responds_in_channel_e2e() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let relay_list = relays();

    // Locate the built binaries (same target dir as this test binary).
    let acp_bin = env!("CARGO_BIN_EXE_sprout-acp");
    // sprout CLI lives next to it in the target dir.
    let target_dir = std::path::Path::new(acp_bin)
        .parent()
        .unwrap()
        .to_path_buf();
    let sprout_bin = target_dir.join("sprout");
    assert!(
        sprout_bin.exists(),
        "sprout CLI not built at {sprout_bin:?} — run `cargo build -p sprout-cli` first"
    );
    let stub = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/stub_agent.sh");

    // Identities: the human (creator) and the agent.
    let human = Keys::generate();
    let agent = Keys::generate();
    let channel = uuid::Uuid::new_v4().to_string();
    let agent_pk = agent.public_key().to_hex();
    let human_pk = human.public_key().to_hex();
    let reply_marker = format!("stub-reply-{}", &channel[..8]);

    eprintln!("channel={channel}\nhuman={human_pk}\nagent={agent_pk}\nrelays={relay_list}");

    // 1. Channel metadata (39000) + members (39002 with BOTH human and agent).
    let meta = EventBuilder::new(Kind::Custom(39000), "")
        .tags(vec![
            nostr::Tag::parse(["d", &channel]).unwrap(),
            nostr::Tag::parse(["name", "e2e-agent-test"]).unwrap(),
            nostr::Tag::parse(["t", "stream"]).unwrap(),
            nostr::Tag::parse(["public"]).unwrap(),
        ])
        .sign_with_keys(&human)
        .unwrap();
    let members = EventBuilder::new(Kind::Custom(39002), "")
        .tags(vec![
            nostr::Tag::parse(["d", &channel]).unwrap(),
            nostr::Tag::parse(["p", &human_pk, "", "owner"]).unwrap(),
            nostr::Tag::parse(["p", &agent_pk, "", "member"]).unwrap(),
        ])
        .sign_with_keys(&human)
        .unwrap();
    // Publish membership to all relays so discovery finds it.
    let meta_ok = publish_all(&relay_list, &meta).await;
    let members_ok = publish_all(&relay_list, &members).await;
    assert!(
        meta_ok && members_ok,
        "no relay accepted channel metadata/membership (meta_ok={meta_ok}, members_ok={members_ok}) — relays may be down/rate-limiting"
    );
    eprintln!("published channel metadata + membership");

    // 2. Spawn the real sprout-acp harness with the stub agent.
    let log_path = std::env::temp_dir().join(format!("acp-e2e-{}.log", &channel[..8]));
    let harness_log_path =
        std::env::temp_dir().join(format!("acp-e2e-harness-{}.log", &channel[..8]));
    let harness_log = std::fs::File::create(&harness_log_path).expect("create harness log");
    // tracing logs go to stdout via `fmt()`; capture both stdout+stderr so the
    // diagnostic dump shows discovery/subscribe/dispatch.
    let harness_log_out = harness_log.try_clone().expect("clone harness log");
    let mut child = Command::new(acp_bin)
        .env("SPROUT_RELAY_URL", &relay_list)
        .env(
            "SPROUT_PRIVATE_KEY",
            agent.secret_key().to_bech32().unwrap(),
        )
        .env("SPROUT_ACP_AGENT_COMMAND", "bash")
        .env("SPROUT_ACP_AGENT_ARGS", stub)
        .env("SPROUT_ACP_RESPOND_TO", "anyone")
        .env("SPROUT_ACP_SUBSCRIBE", "all")
        .env("SPROUT_ACP_NO_MENTION_FILTER", "true")
        .env("SPROUT_ACP_AGENTS", "1")
        .env("STUB_AGENT_CHANNEL", &channel)
        .env("STUB_AGENT_REPLY", &reply_marker)
        .env("STUB_AGENT_SPROUT_BIN", &sprout_bin)
        .env("STUB_AGENT_LOG", &log_path)
        // Hold each turn for 5s so the harness typing-indicator loop (3s tick)
        // fires while a turn is in flight — proves the "agent is typing…" cue.
        .env("STUB_AGENT_PROMPT_DELAY", "5")
        .env("RUST_LOG", "sprout_acp=debug")
        .stdout(Stdio::from(harness_log_out))
        .stderr(Stdio::from(harness_log))
        .kill_on_drop(true)
        .spawn()
        .expect("spawn sprout-acp");

    // Give the harness time to connect to all relays + discover the channel.
    tokio::time::sleep(Duration::from_secs(8)).await;

    // Helper: build + publish a kind-9 @mention from the human.
    let send_mention = |body: &str| {
        let body = body.to_string();
        let channel = channel.clone();
        let agent_pk = agent_pk.clone();
        let human = human.clone();
        let relay_list = relay_list.clone();
        async move {
            let msg = EventBuilder::new(Kind::Custom(9), body)
                .tags(vec![
                    nostr::Tag::parse(["h", &channel]).unwrap(),
                    nostr::Tag::parse(["p", &agent_pk]).unwrap(),
                ])
                .sign_with_keys(&human)
                .unwrap();
            publish_all(&relay_list, &msg).await
        }
    };

    let dump_logs = || {
        if let Ok(h) = std::fs::read_to_string(&harness_log_path) {
            eprintln!("--- harness (sprout-acp) log ---\n{h}\n-----------------------------------");
        }
        if let Ok(log) = std::fs::read_to_string(&log_path) {
            eprintln!("--- stub agent log ---\n{log}\n----------------------");
        }
    };

    // ── Message 1: prove the agent receives + replies, AND emits a typing
    // indicator (kind 20002) while the turn is in flight. Typing events are
    // ephemeral, so we listen live (concurrently) as we send the message. The
    // agent publishes to whichever relay accepts (one may be 503-ing), so we
    // must listen on ALL relays — listening on just one can miss it entirely. ──
    let typing_channel = channel.clone();
    let typing_agent = agent.public_key();
    let typing_relays: Vec<String> = relay_list
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();
    let typing_fut = tokio::spawn(async move {
        let mut handles = Vec::new();
        for relay in typing_relays {
            let ch = typing_channel.clone();
            handles.push(tokio::spawn(async move {
                listen_live(
                    &relay,
                    serde_json::json!({
                        "kinds": [20002],
                        "#h": [ch],
                        "since": (chrono::Utc::now().timestamp() - 2),
                    }),
                    30,
                    move |e| e.pubkey == typing_agent,
                )
                .await
            }));
        }
        // True if ANY relay's listener caught the typing indicator.
        let mut seen = false;
        for h in handles {
            if h.await.unwrap_or(false) {
                seen = true;
            }
        }
        seen
    });

    assert!(
        send_mention("@agent hello, please reply").await,
        "no relay accepted the first @mention"
    );
    eprintln!("published @mention #1; waiting for reply-1…");

    let reply1 = await_reply(
        &relay_list,
        &channel,
        agent.public_key(),
        &format!("{reply_marker}-1"),
        20,
    )
    .await;
    if !reply1 {
        let _ = child.kill().await;
        dump_logs();
        panic!("agent never posted reply #1 (see logs above)");
    }

    // With a 5s turn delay, the harness's 3s typing tick MUST fire while the
    // turn is in flight. This proves the "agent is typing…" cue reaches the
    // relays in serverless (it's published via the nostr-relay-pool backend).
    let typing_seen = typing_fut.await.unwrap_or(false);
    if !typing_seen {
        let _ = child.kill().await;
        dump_logs();
        panic!("no typing indicator (kind 20002) observed during a 5s turn — the 'agent is typing…' cue is broken in serverless");
    }
    eprintln!("✅ typing indicator (kind 20002) observed during turn");

    // ── Message 2: prove the cancel/redispatch path the live GUI hits. A
    // second @mention from the owner triggers OwnerInterrupt (cancel any
    // in-flight turn) then a fresh dispatch. The stub replies per-prompt, so a
    // distinct `-2` reply proves the new turn actually ran. ──
    assert!(
        send_mention("@agent and one more thing").await,
        "no relay accepted the second @mention"
    );
    eprintln!("published @mention #2; waiting for reply-2 (cancel/redispatch path)…");

    let reply2 = await_reply(
        &relay_list,
        &channel,
        agent.public_key(),
        &format!("{reply_marker}-2"),
        20,
    )
    .await;

    let _ = child.kill().await;
    if !reply2 {
        dump_logs();
        panic!("agent never posted reply #2 — cancel/redispatch path is broken (see logs above)");
    }
    eprintln!("✅ both replies landed; cancel/redispatch path works");
}

/// End-to-end test for a PRIVATE (encrypted) channel: the human's message is a
/// NIP-59 gift-wrap (kind 1059) addressed to the agent, wrapping a kind-9 rumor
/// that carries the channel `h` tag — exactly what the desktop publishes for an
/// encrypted channel. This proves the agent:
///   1. subscribes to its gift-wrap inbox (kind 1059, #p=agent),
///   2. decrypts the wrap to recover the inner rumor,
///   3. extracts the channel from the rumor's `h` tag,
///   4. passes the respond gate and replies.
///
/// If this fails while the public test passes, the encrypted-receive path is
/// broken — which is exactly the "goose didn't reply in the private channel"
/// symptom.
#[tokio::test]
#[ignore = "network: hits live public relays; spawns sprout-acp + sprout binaries"]
async fn agent_responds_in_private_channel_e2e() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let relay_list = relays();
    let acp_bin = env!("CARGO_BIN_EXE_sprout-acp");
    let target_dir = std::path::Path::new(acp_bin)
        .parent()
        .unwrap()
        .to_path_buf();
    let sprout_bin = target_dir.join("sprout");
    assert!(
        sprout_bin.exists(),
        "sprout CLI not built at {sprout_bin:?}"
    );
    let stub = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/stub_agent.sh");

    let human = Keys::generate();
    let agent = Keys::generate();
    let channel = uuid::Uuid::new_v4().to_string();
    let agent_pk = agent.public_key().to_hex();
    let human_pk = human.public_key().to_hex();
    let reply_marker = format!("priv-reply-{}", &channel[..8]);

    eprintln!("PRIVATE channel={channel}\nhuman={human_pk}\nagent={agent_pk}");

    // 1. Channel metadata (39000) marked PRIVATE + members (39002).
    let meta = EventBuilder::new(Kind::Custom(39000), "")
        .tags(vec![
            nostr::Tag::parse(["d", &channel]).unwrap(),
            nostr::Tag::parse(["name", "e2e-private-test"]).unwrap(),
            nostr::Tag::parse(["t", "stream"]).unwrap(),
            nostr::Tag::parse(["private"]).unwrap(),
        ])
        .sign_with_keys(&human)
        .unwrap();
    let members = EventBuilder::new(Kind::Custom(39002), "")
        .tags(vec![
            nostr::Tag::parse(["d", &channel]).unwrap(),
            nostr::Tag::parse(["p", &human_pk, "", "owner"]).unwrap(),
            nostr::Tag::parse(["p", &agent_pk, "", "member"]).unwrap(),
        ])
        .sign_with_keys(&human)
        .unwrap();
    assert!(
        publish_all(&relay_list, &meta).await && publish_all(&relay_list, &members).await,
        "no relay accepted private channel metadata/membership"
    );
    eprintln!("published private channel metadata + membership");

    // 2. Spawn the harness (same config as the public test).
    let log_path = format!("/tmp/acp-e2e-priv-agent-{}.log", &channel[..8]);
    let harness_log_path = format!("/tmp/acp-e2e-priv-harness-{}.log", &channel[..8]);
    let _ = std::fs::remove_file(&log_path);
    let _ = std::fs::remove_file(&harness_log_path);
    let harness_log = std::fs::File::create(&harness_log_path).unwrap();
    let harness_log_out = harness_log.try_clone().unwrap();
    // NOTE: no SPROUT_AUTH_TAG — `respond_to=anyone` doesn't need an owner, and
    // a bogus tag would make the stub's inherited `sprout messages send` fail
    // auth. (The owner gate is exercised separately.)

    let mut child = Command::new(acp_bin)
        .env("SPROUT_RELAY_URL", &relay_list)
        .env(
            "SPROUT_PRIVATE_KEY",
            agent.secret_key().to_bech32().unwrap(),
        )
        .env("SPROUT_SERVERLESS", "true")
        .env("SPROUT_ACP_AGENT_COMMAND", "bash")
        .env("SPROUT_ACP_AGENT_ARGS", stub)
        .env("SPROUT_ACP_RESPOND_TO", "anyone")
        .env("SPROUT_ACP_SUBSCRIBE", "all")
        .env("SPROUT_ACP_NO_MENTION_FILTER", "true")
        .env("SPROUT_ACP_AGENTS", "1")
        .env("STUB_AGENT_CHANNEL", &channel)
        .env("STUB_AGENT_REPLY", &reply_marker)
        .env("STUB_AGENT_SPROUT_BIN", &sprout_bin)
        .env("STUB_AGENT_LOG", &log_path)
        .env("RUST_LOG", "sprout_acp=debug")
        .stdout(Stdio::from(harness_log_out))
        .stderr(Stdio::from(harness_log))
        .kill_on_drop(true)
        .spawn()
        .expect("spawn sprout-acp");

    // Let the harness connect + subscribe its gift-wrap inbox + discover.
    tokio::time::sleep(Duration::from_secs(10)).await;

    // 3. Send an ENCRYPTED message: a kind-9 rumor (with the channel `h` tag),
    // gift-wrapped to the agent. This is exactly what the desktop publishes for
    // a private channel — no plaintext kind-9 ever hits the relay.
    let rumor = EventBuilder::new(Kind::Custom(9), "@agent private hello, please reply")
        .tags(vec![
            nostr::Tag::parse(["h", &channel]).unwrap(),
            nostr::Tag::parse(["p", &agent_pk]).unwrap(),
        ])
        .build(human.public_key());
    let wrap = EventBuilder::gift_wrap(&human, &agent.public_key(), rumor, [])
        .await
        .expect("build gift wrap");
    assert_eq!(wrap.kind, Kind::Custom(1059), "must be a gift wrap");
    assert!(
        publish_all(&relay_list, &wrap).await,
        "no relay accepted the gift wrap"
    );
    eprintln!("published gift-wrapped @mention; waiting for reply…");

    // 4. The reply lands as a plaintext kind-9 (the stub replies plainly; the
    // point is proving the agent RECEIVED + DECRYPTED the private message).
    let found = await_reply(&relay_list, &channel, agent.public_key(), &reply_marker, 20).await;

    let _ = child.kill().await;
    if !found {
        if let Ok(h) = std::fs::read_to_string(&harness_log_path) {
            eprintln!("--- harness log ---\n{h}\n-------------------");
        }
        if let Ok(l) = std::fs::read_to_string(&log_path) {
            eprintln!("--- stub log ---\n{l}\n----------------");
        }
        panic!("agent never replied to the PRIVATE (gift-wrapped) message — encrypted-receive path is broken");
    }
    eprintln!("✅ agent received + decrypted the private message and replied");
}

/// DM namespace — MUST match `desktop/src-tauri/src/commands/dms.rs`.
const DM_NAMESPACE_BYTES: [u8; 16] = [
    0x6f, 0x1d, 0x2c, 0x3b, 0x4a, 0x59, 0x4e, 0x87, 0x9b, 0x0c, 0x1d, 0x2e, 0x3f, 0x4a, 0x5b, 0x6c,
];

/// Derive the DM channel id the same way the desktop does: UUIDv5 over the
/// sorted, lowercased, de-duplicated participant set.
fn derive_dm_channel_id(participants: &[String]) -> String {
    let mut p: Vec<String> = participants
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    p.sort();
    p.dedup();
    let ns = uuid::Uuid::from_bytes(DM_NAMESPACE_BYTES);
    uuid::Uuid::new_v5(&ns, p.join(",").as_bytes()).to_string()
}

/// End-to-end test for a DIRECT MESSAGE (1:1, encrypted). A DM in serverless is
/// a private channel with a deterministic UUIDv5 id derived from the two
/// participants and `t=dm`. The message is gift-wrapped (kind 1059) to the
/// agent — exactly what the desktop publishes when you DM an agent. Proves the
/// agent receives + decrypts a DM and replies.
#[tokio::test]
#[ignore = "network: hits live public relays; spawns sprout-acp + sprout binaries"]
async fn agent_responds_in_dm_e2e() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let relay_list = relays();
    let acp_bin = env!("CARGO_BIN_EXE_sprout-acp");
    let target_dir = std::path::Path::new(acp_bin)
        .parent()
        .unwrap()
        .to_path_buf();
    let sprout_bin = target_dir.join("sprout");
    assert!(
        sprout_bin.exists(),
        "sprout CLI not built at {sprout_bin:?}"
    );
    let stub = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/stub_agent.sh");

    let human = Keys::generate();
    let agent = Keys::generate();
    let agent_pk = agent.public_key().to_hex();
    let human_pk = human.public_key().to_hex();

    // DM channel id = UUIDv5 over {human, agent} — both sides converge on this.
    let channel = derive_dm_channel_id(&[human_pk.clone(), agent_pk.clone()]);
    let reply_marker = format!("dm-reply-{}", &channel[..8]);
    let participants = {
        let mut p = vec![human_pk.clone(), agent_pk.clone()];
        p.sort();
        p
    };

    eprintln!("DM channel={channel}\nhuman={human_pk}\nagent={agent_pk}");

    // 1. DM channel metadata (39000, t=dm, private) + members (39002).
    let meta = EventBuilder::new(Kind::Custom(39000), "")
        .tags(vec![
            nostr::Tag::parse(["d", &channel]).unwrap(),
            nostr::Tag::parse(["name", "Direct message"]).unwrap(),
            nostr::Tag::parse(["t", "dm"]).unwrap(),
            nostr::Tag::parse(["private"]).unwrap(),
        ])
        .sign_with_keys(&human)
        .unwrap();
    let members = EventBuilder::new(Kind::Custom(39002), "")
        .tags(vec![
            nostr::Tag::parse(["d", &channel]).unwrap(),
            nostr::Tag::parse(["p", &participants[0], "", "owner"]).unwrap(),
            nostr::Tag::parse(["p", &participants[1], "", "member"]).unwrap(),
        ])
        .sign_with_keys(&human)
        .unwrap();
    assert!(
        publish_all(&relay_list, &meta).await && publish_all(&relay_list, &members).await,
        "no relay accepted DM metadata/membership"
    );
    eprintln!("published DM channel metadata + membership");

    // 2. Spawn the harness.
    let log_path = format!("/tmp/acp-e2e-dm-agent-{}.log", &channel[..8]);
    let harness_log_path = format!("/tmp/acp-e2e-dm-harness-{}.log", &channel[..8]);
    let _ = std::fs::remove_file(&log_path);
    let _ = std::fs::remove_file(&harness_log_path);
    let harness_log = std::fs::File::create(&harness_log_path).unwrap();
    let harness_log_out = harness_log.try_clone().unwrap();

    let mut child = Command::new(acp_bin)
        .env("SPROUT_RELAY_URL", &relay_list)
        .env(
            "SPROUT_PRIVATE_KEY",
            agent.secret_key().to_bech32().unwrap(),
        )
        .env("SPROUT_SERVERLESS", "true")
        .env("SPROUT_ACP_AGENT_COMMAND", "bash")
        .env("SPROUT_ACP_AGENT_ARGS", stub)
        .env("SPROUT_ACP_RESPOND_TO", "anyone")
        .env("SPROUT_ACP_SUBSCRIBE", "all")
        .env("SPROUT_ACP_NO_MENTION_FILTER", "true")
        .env("SPROUT_ACP_AGENTS", "1")
        .env("STUB_AGENT_CHANNEL", &channel)
        .env("STUB_AGENT_REPLY", &reply_marker)
        .env("STUB_AGENT_SPROUT_BIN", &sprout_bin)
        .env("STUB_AGENT_LOG", &log_path)
        .env("RUST_LOG", "sprout_acp=debug")
        .stdout(Stdio::from(harness_log_out))
        .stderr(Stdio::from(harness_log))
        .kill_on_drop(true)
        .spawn()
        .expect("spawn sprout-acp");

    tokio::time::sleep(Duration::from_secs(10)).await;

    // 3. Send the DM: a kind-9 rumor (h = DM channel id), gift-wrapped to agent.
    let rumor = EventBuilder::new(Kind::Custom(9), "@agent hey, this is a DM — reply please")
        .tags(vec![
            nostr::Tag::parse(["h", &channel]).unwrap(),
            nostr::Tag::parse(["p", &agent_pk]).unwrap(),
        ])
        .build(human.public_key());
    let wrap = EventBuilder::gift_wrap(&human, &agent.public_key(), rumor, [])
        .await
        .expect("build gift wrap");
    assert_eq!(wrap.kind, Kind::Custom(1059), "DM must be gift-wrapped");
    assert!(
        publish_all(&relay_list, &wrap).await,
        "no relay accepted the DM gift wrap"
    );
    eprintln!("published gift-wrapped DM; waiting for reply…");

    // 4. Assert the agent replied in the DM channel.
    let found = await_reply(&relay_list, &channel, agent.public_key(), &reply_marker, 20).await;

    let _ = child.kill().await;
    if !found {
        if let Ok(h) = std::fs::read_to_string(&harness_log_path) {
            eprintln!("--- harness log ---\n{h}\n-------------------");
        }
        if let Ok(l) = std::fs::read_to_string(&log_path) {
            eprintln!("--- stub log ---\n{l}\n----------------");
        }
        panic!("agent never replied to the DM — DM receive path is broken");
    }
    eprintln!("✅ agent received + decrypted the DM and replied");
}
