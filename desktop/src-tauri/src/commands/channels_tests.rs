// Tests for commands/channels.rs — split into a sibling file to keep
// channels.rs under the per-file line cap.

use super::*;
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

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

/// Like [`ev`] but with an explicit `created_at` and a caller-supplied signing
/// key — so a test can produce multiple versions of the *same* addressable
/// event (same author + d-tag) with different timestamps to exercise
/// "latest wins" resolution.
fn ev_at(keys: &Keys, kind: u16, created_at: u64, tags: Vec<Vec<&str>>) -> nostr::Event {
    let parsed: Vec<Tag> = tags
        .into_iter()
        .map(|t| Tag::parse(t).expect("parse tag"))
        .collect();
    EventBuilder::new(Kind::from_u16(kind), "")
        .tags(parsed)
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
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
        "member",
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
        final_members
            .iter()
            .any(|(pk, _)| pk == &creator_pk.to_ascii_lowercase()),
        "creator missing from member list after joiner joined — \
         read-modify-write clobbered the creator (the real bug?)"
    );
    assert!(
        final_members
            .iter()
            .any(|(pk, _)| pk == &joiner_pk.to_ascii_lowercase()),
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
        mems.iter().any(|(m, _)| m == &pk.to_ascii_lowercase()),
        "membership not found across relays"
    );

    eprintln!("✅ multi-relay fanout OK: published to 2, read+deduped from 2");
}

// ── Connection-reuse burst test (the rate-limit fix) ─────────────────────────
//
// Reproduces the real-app failure: a single AppState (one pooled connection)
// firing many ops in quick succession — like get_channels (~10 queries) +
// create channel (2 publishes) + several messages. Before the connection pool,
// each op opened a fresh WebSocket and public relays rate-limited the storm
// ("you are noting too much"). With the pool, all ops reuse ONE socket per
// relay, so the burst goes through.
//
// Run with:
//   cargo test --manifest-path desktop/src-tauri/Cargo.toml \
//     serverless_burst_no_rate_limit -- --ignored --nocapture
#[tokio::test]
#[ignore = "network: hits a live public relay (RELAY_URL, default damus)"]
async fn serverless_burst_no_rate_limit() {
    use crate::app_state::build_app_state;
    use crate::relay::{query_relay, submit_event};
    use std::sync::atomic::Ordering;

    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let keys = nostr::Keys::generate();
    let pk = keys.public_key().to_hex();
    let relay = std::env::var("RELAY_URL").unwrap_or_else(|_| "wss://relay.damus.io".to_string());

    let state = build_app_state();
    *state.keys.lock().unwrap() = keys.clone();
    *state.relay_url_override.lock().unwrap() = Some(relay.clone());
    state.serverless.store(true, Ordering::Relaxed);

    // Burst of ~15 sequential queries (mimics get_channels firing many REQs).
    for i in 0..15 {
        query_relay(
            &state,
            &[serde_json::json!({"kinds":[39000],"#d":[format!("burst-{i}")],"limit":1})],
        )
        .await
        .unwrap_or_else(|e| panic!("query {i} failed (rate-limited?): {e}"));
    }
    eprintln!("✅ 15 sequential queries reused one pooled connection");

    // Burst of publishes (mimics create channel + messages). All must be
    // accepted — a rate-limit rejection here is the bug we fixed.
    for i in 0..6 {
        let channel = uuid::Uuid::new_v4();
        let builder = crate::events::build_message(
            channel,
            &format!("burst message {i} from {}", &pk[..8]),
            None,
            &[],
            &[],
            &[],
            &[],
        )
        .expect("build message");
        let resp = submit_event(builder, &state)
            .await
            .unwrap_or_else(|e| panic!("publish {i} failed (rate-limited?): {e}"));
        assert!(
            !resp.message.contains("rate-limit"),
            "publish {i} was rate-limited — connection pool not reusing socket: {}",
            resp.message
        );
    }
    eprintln!("✅ 6 sequential publishes reused one pooled connection — no rate limit");
}

// ── Multi-relay live subscription (the split-brain fix) ──────────────────────
//
// Reproduces the exact bug: a message published to relay B must still reach a
// subscriber, because the subscription is open on ALL relays at once (standard
// Nostr — like damus/SimplePool), not just one. We:
//   1. open a pool live subscription across [damus, nos.lol] for a channel,
//   2. publish a kind-9 message to that channel (lands on whichever accepts),
//   3. assert the subscription delivers it within a few seconds.
//
// Run with:
//   cargo test --manifest-path desktop/src-tauri/Cargo.toml \
//     serverless_live_subscription_multi_relay -- --ignored --nocapture
#[tokio::test]
#[ignore = "network: hits live public relays"]
async fn serverless_live_subscription_multi_relay() {
    use crate::app_state::build_app_state;
    use crate::relay::submit_event;
    use std::sync::atomic::Ordering;

    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let keys = nostr::Keys::generate();
    let relays: Vec<String> = std::env::var("RELAY_URL")
        .unwrap_or_else(|_| "wss://relay.damus.io,wss://nos.lol".to_string())
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();

    let state = build_app_state();
    *state.keys.lock().unwrap() = keys.clone();
    *state.relay_url_override.lock().unwrap() = Some(relays.join(","));
    state.serverless.store(true, Ordering::Relaxed);

    let channel = uuid::Uuid::new_v4();
    let secret = format!("live-sub-{}", &channel.to_string()[..8]);

    // 1. Open a live subscription across all relays.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<nostr::Event>();
    let sub_id = state
        .relay_pool
        .subscribe(
            &relays,
            &keys,
            serde_json::json!({
                "kinds": [9],
                "#h": [channel.to_string()],
                "since": chrono::Utc::now().timestamp() - 5,
            }),
            tx,
        )
        .await;
    eprintln!("opened live sub {sub_id} across {relays:?}");

    // Let the REQs register on all relays.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // 2. Publish a message (multi-relay fanout — lands wherever accepts).
    let builder = crate::events::build_message(channel, &secret, None, &[], &[], &[], &[])
        .expect("build message");
    let resp = submit_event(builder, &state).await.expect("publish");
    eprintln!(
        "published: accepted={} msg={:?}",
        resp.accepted, resp.message
    );

    // 3. The live subscription must deliver it (from whichever relay stored it).
    let got = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while let Some(ev) = rx.recv().await {
            if ev.content == secret {
                return Some(ev);
            }
        }
        None
    })
    .await
    .expect("timed out waiting for live event — split-brain not fixed");

    let got = got.expect("subscription channel closed before event arrived");
    assert_eq!(got.content, secret);
    assert_eq!(got.pubkey, keys.public_key());
    eprintln!("✅ live subscription delivered the message across relays — no split-brain");

    state.relay_pool.unsubscribe(&sub_id).await;
}

// ── Replaceable-event "latest wins" resolution ───────────────────────────────
//
// Regression coverage for the multi-relay membership bug: when relays hold
// divergent copies of a replaceable kind:39002 (one relay dropped a write and
// kept a stale copy), the client must resolve to the LATEST copy by
// `created_at`. Picking an arbitrary copy made the member list flicker and made
// read-modify-write membership updates silently clobber members.

const PK_BOT: &str = "2b5f20697e34f75726f567dcc6657b7ca4a10afc5d341ec50f34d91fd3014874";

#[test]
fn latest_event_picks_newest_by_created_at() {
    let keys = Keys::generate();
    // Stale copy: 2 members. Fresh copy: 3 members (bot added later).
    let stale = ev_at(
        &keys,
        39002,
        1000,
        vec![
            vec!["d", "chan"],
            vec!["p", PK_A, "", "member"],
            vec!["p", PK_B, "", "member"],
        ],
    );
    let fresh = ev_at(
        &keys,
        39002,
        2000,
        vec![
            vec!["d", "chan"],
            vec!["p", PK_A, "", "member"],
            vec!["p", PK_B, "", "member"],
            vec!["p", PK_BOT, "", "member"],
        ],
    );

    // Whatever order relays return them in, the newest must win.
    for events in [
        vec![stale.clone(), fresh.clone()],
        vec![fresh.clone(), stale.clone()],
    ] {
        let chosen = latest_event(&events).expect("some event");
        assert_eq!(
            chosen.id, fresh.id,
            "must pick the newest copy regardless of input order"
        );
        let resp = nostr_convert::channel_members_from_event(chosen).expect("parse members");
        let pks: Vec<&str> = resp.members.iter().map(|m| m.pubkey.as_str()).collect();
        assert!(
            pks.contains(&PK_BOT),
            "bot added in the newer copy must be present, got {pks:?}"
        );
        assert_eq!(resp.members.len(), 3);
    }
}

#[test]
fn latest_event_empty_is_none() {
    assert!(latest_event(&[]).is_none());
}

#[test]
fn latest_event_tie_break_is_deterministic() {
    // Two copies with the SAME created_at must resolve deterministically
    // (highest event id), not by chance/order.
    let k1 = Keys::generate();
    let k2 = Keys::generate();
    let a = ev_at(
        &k1,
        39002,
        5000,
        vec![vec!["d", "chan"], vec!["p", PK_A, "", "member"]],
    );
    let b = ev_at(
        &k2,
        39002,
        5000,
        vec![vec!["d", "chan"], vec!["p", PK_B, "", "member"]],
    );
    let expected = if a.id.to_hex() > b.id.to_hex() {
        a.id
    } else {
        b.id
    };
    assert_eq!(latest_event(&[a.clone(), b.clone()]).unwrap().id, expected);
    assert_eq!(latest_event(&[b, a]).unwrap().id, expected);
}

#[test]
fn latest_by_d_tag_resolves_each_channel_independently() {
    let k = Keys::generate();
    // chan-1: stale (1 member) + fresh (2 members). chan-2: single copy.
    let c1_stale = ev_at(
        &k,
        39002,
        1000,
        vec![vec!["d", "chan-1"], vec!["p", PK_A, "", "member"]],
    );
    let c1_fresh = ev_at(
        &k,
        39002,
        2000,
        vec![
            vec!["d", "chan-1"],
            vec!["p", PK_A, "", "member"],
            vec!["p", PK_BOT, "", "member"],
        ],
    );
    let c2 = ev_at(
        &k,
        39002,
        1500,
        vec![vec!["d", "chan-2"], vec!["p", PK_C, "", "member"]],
    );

    let events = [c1_stale, c2.clone(), c1_fresh.clone()];
    let map = latest_by_d_tag(&events);
    assert_eq!(map.len(), 2);
    assert_eq!(
        map.get("chan-1").unwrap().id,
        c1_fresh.id,
        "chan-1 must be the fresh copy"
    );
    assert_eq!(map.get("chan-2").unwrap().id, c2.id);
}

#[test]
fn latest_by_d_tag_skips_events_without_d_tag() {
    let k = Keys::generate();
    let no_d = ev_at(&k, 39002, 1000, vec![vec!["p", PK_A, "", "member"]]);
    let with_d = ev_at(
        &k,
        39002,
        1000,
        vec![vec!["d", "chan"], vec!["p", PK_B, "", "member"]],
    );
    let events = [no_d, with_d.clone()];
    let map = latest_by_d_tag(&events);
    assert_eq!(map.len(), 1);
    assert_eq!(map.get("chan").unwrap().id, with_d.id);
}

#[test]
fn collect_members_by_channel_uses_latest_copy() {
    // The batch path (channel browser member counts) must also resolve to the
    // latest copy — a stale copy must not undercount members.
    let k = Keys::generate();
    let stale = ev_at(
        &k,
        39002,
        1000,
        vec![vec!["d", "chan-x"], vec!["p", PK_A, "", "member"]],
    );
    let fresh = ev_at(
        &k,
        39002,
        2000,
        vec![
            vec!["d", "chan-x"],
            vec!["p", PK_A, "", "member"],
            vec!["p", PK_BOT, "", "member"],
        ],
    );
    // Stale listed AFTER fresh — naive last-write-wins iteration would have
    // picked the stale one; latest_by_d_tag must still choose fresh.
    let map = collect_members_by_channel(&[fresh, stale]);
    let info = map.get("chan-x").expect("channel present");
    assert_eq!(
        info.count, 2,
        "must count the fresh 2-member copy, not stale 1-member"
    );
    assert!(info.pubkeys.iter().any(|p| p == PK_BOT));
}
