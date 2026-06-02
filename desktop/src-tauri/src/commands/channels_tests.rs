// Tests for commands/channels.rs — split into a sibling file to keep
// channels.rs under the per-file line cap.

use super::*;
use nostr::{EventBuilder, Keys, Kind, Tag};

/// Build a signed event for testing with the given kind, content, and tags.
fn ev(kind: u16, content: &str, tags: Vec<Vec<&str>>) -> nostr::Event {
    let keys = Keys::generate();
    let parsed: Vec<Tag> = tags
        .into_iter()
        .map(|t| Tag::parse(t).expect("parse tag"))
        .collect();
    EventBuilder::new(Kind::from_u16(kind), content)
        .tags(parsed)
        .sign_with_keys(&keys)
        .expect("sign")
}

// A 64-hex pubkey (nostr p-tags require 32-byte hex).
const PK_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const PK_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const PK_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

#[test]
fn counts_unique_p_tags_per_channel() {
    let e1 = ev(
        39002,
        "",
        vec![
            vec!["d", "chan-1"],
            vec!["p", PK_A, "", "member"],
            vec!["p", PK_B, "", "admin"],
        ],
    );
    let e2 = ev(
        39002,
        "",
        vec![vec!["d", "chan-2"], vec!["p", PK_C, "", "member"]],
    );

    let membership = collect_members_by_channel(&[e1, e2]);
    assert_eq!(membership.get("chan-1").map(|m| m.count), Some(2));
    assert_eq!(membership.get("chan-2").map(|m| m.count), Some(1));
    assert_eq!(membership.len(), 2);

    let mut pks: Vec<&str> = membership["chan-1"]
        .pubkeys
        .iter()
        .map(|s| s.as_str())
        .collect();
    pks.sort();
    assert_eq!(pks, vec![PK_A, PK_B]);
}

#[test]
fn dedupes_repeated_pubkeys() {
    let e = ev(
        39002,
        "",
        vec![
            vec!["d", "chan-1"],
            vec!["p", PK_A, "", "member"],
            vec!["p", PK_A, "", "admin"], // duplicate pubkey, different role
            vec!["p", PK_B, "", "member"],
        ],
    );
    let membership = collect_members_by_channel(&[e]);
    assert_eq!(membership.get("chan-1").map(|m| m.count), Some(2));
}

#[test]
fn skips_event_without_d_tag() {
    let e = ev(39002, "", vec![vec!["p", PK_A, "", "member"]]);
    let membership = collect_members_by_channel(&[e]);
    assert!(membership.is_empty());
}

#[test]
fn zero_member_channel_is_recorded() {
    // A channel with a members event but no p-tags should report 0,
    // not be absent from the map (the caller relies on `get` returning
    // `Some(0)` to overwrite a default).
    let e = ev(39002, "", vec![vec!["d", "chan-1"]]);
    let membership = collect_members_by_channel(&[e]);
    assert_eq!(membership.get("chan-1").map(|m| m.count), Some(0));
    assert!(membership["chan-1"].pubkeys.is_empty());
}

#[test]
fn empty_input_yields_empty_map() {
    let membership = collect_members_by_channel(&[]);
    assert!(membership.is_empty());
}

// ── Serverless integration test (hits a real public relay) ───────────────────
//
// Run with:
//   cargo test --manifest-path desktop/src-tauri/Cargo.toml \
//     -- --ignored --nocapture serverless_create_join_roundtrip
//
// Drives the EXACT serverless code paths (query_relay/submit_event over WS +
// the 39000/39002 builders + serverless_set_members read-modify-write) against
// wss://relay.damus.io to reproduce the "join does nothing" bug.

#[tokio::test]
#[ignore = "network: hits wss://relay.damus.io"]
async fn serverless_create_join_roundtrip() {
    use crate::app_state::build_app_state;
    use crate::relay::{query_relay, submit_event};
    use std::sync::atomic::Ordering;

    // The app installs a rustls CryptoProvider via tauri/wry at startup; tests
    // don't, and both aws-lc-rs and ring are present (ambiguous). Pick one.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    // Two independent identities: a creator and a joiner.
    let creator_keys = nostr::Keys::generate();
    let joiner_keys = nostr::Keys::generate();
    let creator_pk = creator_keys.public_key().to_hex();
    let joiner_pk = joiner_keys.public_key().to_hex();
    eprintln!("creator={creator_pk}\njoiner={joiner_pk}");

    let relay = std::env::var("RELAY_URL").unwrap_or_else(|_| "wss://relay.damus.io".to_string());
    let relay = relay.as_str();

    // Build a serverless AppState for the CREATOR.
    let creator_state = build_app_state();
    *creator_state.keys.lock().unwrap() = creator_keys.clone();
    *creator_state.relay_url_override.lock().unwrap() = Some(relay.to_string());
    creator_state.serverless.store(true, Ordering::Relaxed);

    // ── Step 1: create a channel (publish 39000 + 39002 self) ──────────────
    let channel_id = uuid::Uuid::new_v4().to_string();
    let name = format!("it-{}", &channel_id[..8]);
    eprintln!("creating channel {channel_id} ({name})");

    let meta = events::build_channel_metadata_serverless(
        &channel_id,
        &name,
        "open",
        "stream",
        Some("integration test"),
        &[],
    )
    .expect("build metadata");
    let r1 = submit_event(meta, &creator_state)
        .await
        .expect("publish 39000");
    eprintln!("39000 publish: accepted={} msg={}", r1.accepted, r1.message);

    let members =
        events::build_channel_members_serverless(&channel_id, std::slice::from_ref(&creator_pk))
            .expect("build members");
    let r2 = submit_event(members, &creator_state)
        .await
        .expect("publish 39002");
    eprintln!("39002 publish: accepted={} msg={}", r2.accepted, r2.message);

    // Give the relay a moment to index.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // ── Diagnostics: is the 39002 readable at all, and by which filter? ────
    let by_d = query_relay(
        &creator_state,
        &[serde_json::json!({"kinds":[39002],"#d":[channel_id],"limit":10})],
    )
    .await
    .expect("query 39002 by #d");
    eprintln!("39002 by #d only: {} event(s)", by_d.len());
    if let Some(ev) = by_d.first() {
        eprintln!("  39002 author = {}", ev.pubkey.to_hex());
        eprintln!(
            "  39002 tags   = {:?}",
            ev.tags
                .iter()
                .map(|t| t.as_slice().to_vec())
                .collect::<Vec<_>>()
        );
        eprintln!("  creator_pk   = {creator_pk}");
    }

    let by_kind_author = query_relay(
        &creator_state,
        &[serde_json::json!({"kinds":[39002],"authors":[creator_pk],"limit":10})],
    )
    .await
    .expect("query 39002 by author");
    eprintln!("39002 by author: {} event(s)", by_kind_author.len());

    let meta_by_d = query_relay(
        &creator_state,
        &[serde_json::json!({"kinds":[39000],"#d":[channel_id],"limit":10})],
    )
    .await
    .expect("query 39000 by #d");
    eprintln!("39000 by #d (control): {} event(s)", meta_by_d.len());

    // ── Step 2: read it back as the creator (should be a member) ───────────
    let creator_member_events = query_relay(
        &creator_state,
        &[serde_json::json!({"kinds":[39002],"#p":[creator_pk],"#d":[channel_id],"limit":10})],
    )
    .await
    .expect("query creator membership");
    eprintln!(
        "creator sees {} membership event(s)",
        creator_member_events.len()
    );
    assert!(
        !creator_member_events.is_empty(),
        "BUG: creator's own 39002 membership not found after create — \
         either the publish was rejected or the read filter is wrong"
    );

    // ── Step 3: JOINER joins (read-modify-write of 39002) ──────────────────
    let joiner_state = build_app_state();
    *joiner_state.keys.lock().unwrap() = joiner_keys.clone();
    *joiner_state.relay_url_override.lock().unwrap() = Some(relay.to_string());
    joiner_state.serverless.store(true, Ordering::Relaxed);

    // This is exactly what join_channel does in serverless mode.
    serverless_set_members(
        &joiner_state,
        &channel_id,
        std::slice::from_ref(&joiner_pk),
        &[],
    )
    .await
    .expect("join (set members)");
    eprintln!("joiner published updated membership");

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // ── Step 4: read membership — BOTH should be present ───────────────────
    let final_members = serverless_current_members(&joiner_state, &channel_id)
        .await
        .expect("read final members");
    eprintln!("final members ({}): {final_members:?}", final_members.len());

    assert!(
        final_members.contains(&creator_pk.to_ascii_lowercase()),
        "creator missing from member list after joiner joined — \
         read-modify-write clobbered the creator (the real bug?)"
    );
    assert!(
        final_members.contains(&joiner_pk.to_ascii_lowercase()),
        "joiner missing from member list after join — join didn't persist"
    );

    // ── Step 5: joiner's get_channels-style membership lookup ──────────────
    let joiner_member_events = query_relay(
        &joiner_state,
        &[serde_json::json!({"kinds":[39002],"#p":[joiner_pk],"limit":50})],
    )
    .await
    .expect("query joiner membership");
    let joined_this_channel = joiner_member_events.iter().any(|ev| {
        ev.tags.iter().any(|t| {
            let s = t.as_slice();
            s.len() >= 2 && s[0] == "d" && s[1] == channel_id
        })
    });
    assert!(
        joined_this_channel,
        "BUG REPRODUCED: after join, get_channels' #p-filtered 39002 query \
         does not return this channel for the joiner → UI still shows \
         'join to participate'"
    );

    eprintln!("✅ roundtrip OK: create → join → both members visible");
}

#[tokio::test]
#[ignore = "network: hits multiple public relays"]
async fn serverless_multi_relay_fanout() {
    use crate::app_state::build_app_state;
    use crate::relay::{query_relay, submit_event};
    use std::sync::atomic::Ordering;

    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let keys = nostr::Keys::generate();
    let pk = keys.public_key().to_hex();

    // Two relays. Publish fans out to both; reads merge+dedup from both.
    let relays = std::env::var("RELAY_URL")
        .unwrap_or_else(|_| "wss://relay.damus.io,wss://nos.lol".to_string());
    let relays = relays.as_str();

    let state = build_app_state();
    *state.keys.lock().unwrap() = keys.clone();
    *state.relay_url_override.lock().unwrap() = Some(relays.to_string());
    state.serverless.store(true, Ordering::Relaxed);

    let channel_id = uuid::Uuid::new_v4().to_string();
    let name = format!("multi-{}", &channel_id[..8]);

    let meta =
        events::build_channel_metadata_serverless(&channel_id, &name, "open", "stream", None, &[])
            .unwrap();
    let r = submit_event(meta, &state)
        .await
        .expect("publish to all relays");
    eprintln!(
        "multi-relay publish: accepted={} msg={}",
        r.accepted, r.message
    );
    assert!(r.accepted);

    let members =
        events::build_channel_members_serverless(&channel_id, std::slice::from_ref(&pk)).unwrap();
    submit_event(members, &state)
        .await
        .expect("publish members");

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Read across both relays — should find the channel, deduped to one event.
    let metas = query_relay(
        &state,
        &[serde_json::json!({"kinds":[39000],"#d":[channel_id],"limit":10})],
    )
    .await
    .expect("query metadata");
    eprintln!("39000 (deduped across 2 relays): {} event(s)", metas.len());
    assert_eq!(
        metas.len(),
        1,
        "expected exactly one deduped metadata event"
    );

    let mems = serverless_current_members(&state, &channel_id)
        .await
        .expect("read members");
    assert!(
        mems.contains(&pk.to_ascii_lowercase()),
        "membership not found across relays"
    );

    eprintln!("✅ multi-relay fanout OK: published to 2, read+deduped from 2");
}
