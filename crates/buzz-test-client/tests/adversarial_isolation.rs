//! Adversarial multi-tenant isolation battery — blue-team control verification.
//!
//! Independent, wire-level verification of the relay's core security invariant:
//! **all tenant-observable state under a URL is community-local** (README).
//! Two communities (a.localhost, b.localhost) are served by ONE relay process,
//! one Postgres, one Redis; only the `Host` header differs. Each probe below
//! plants tenant-observable state in community A and then attempts to observe
//! it from community B, asserting the fence holds (OWASP Top 10:2025 A01 —
//! Broken Access Control; CWE-668 — Exposure of Resource to Wrong Sphere;
//! CWE-200 — Exposure of Sensitive Information for oracle-shaped errors).
//!
//! Every probe also asserts the CONTROL side (A sees its own state) so a
//! "fence holds" result can never be faked by the state simply not existing —
//! the false-zero guard: a probe only counts if its control arm observed the
//! planted state in A.
//!
//! Relationship to `conformance_multitenant.rs`: that file is the team's
//! obligation table (several rows `todo!()`-stubbed pending lanes). This file
//! is an INDEPENDENT adversarial battery — it does not share helpers or
//! assumptions, and it deliberately re-probes some stubbed lanes
//! (event-id cross-fetch, DM fanout) because a stub means "unverified", not
//! "verified safe".
//!
//! # Running
//!
//! One relay process with two host mappings (see `communities` table):
//!
//! ```text
//! RELAY_URL_A=ws://a.localhost:3000 RELAY_URL_B=ws://b.localhost:3000 \
//! cargo test -p buzz-test-client --test adversarial_isolation -- --ignored
//! ```

use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use buzz_test_client::BuzzTestClient;
use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag, Timestamp};
use sha2::{Digest, Sha256};

fn url_a() -> String {
    std::env::var("RELAY_URL_A").unwrap_or_else(|_| "ws://a.localhost:3000".to_string())
}

fn url_b() -> String {
    std::env::var("RELAY_URL_B").unwrap_or_else(|_| "ws://b.localhost:3000".to_string())
}

fn http_a() -> String {
    url_a()
        .replace("wss://", "https://")
        .replace("ws://", "http://")
        .trim_end_matches('/')
        .to_string()
}

fn http_b() -> String {
    url_b()
        .replace("wss://", "https://")
        .replace("ws://", "http://")
        .trim_end_matches('/')
        .to_string()
}

fn sub_id(name: &str) -> String {
    format!("adv-iso-{name}-{}", uuid::Uuid::new_v4())
}

/// Submit a signed event to `http_base`'s `POST /events` bridge (dev mode:
/// `BUZZ_REQUIRE_AUTH_TOKEN=false` → `x-pubkey` header stands in for NIP-98).
async fn submit_event(http_base: &str, keys: &Keys, event: &nostr::Event) -> serde_json::Value {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{http_base}/events"))
        .header("X-Pubkey", keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(event).unwrap())
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST /events to {http_base} failed: {e}"));
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.expect("parse /events response");
    assert!(
        status.is_success(),
        "POST /events to {http_base} returned {status}: {body}"
    );
    body
}

/// Create an `open` channel (kind:9007) in the community bound to `http_base`.
/// Returns the channel UUID string.
async fn create_channel(http_base: &str, keys: &Keys) -> String {
    let channel_uuid = uuid::Uuid::new_v4();
    let event = EventBuilder::new(Kind::Custom(9007), "")
        .tags(vec![
            Tag::parse(["h", &channel_uuid.to_string()]).unwrap(),
            Tag::parse(["name", &format!("adv-iso-{channel_uuid}")]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "open"]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();
    let body = submit_event(http_base, keys, &event).await;
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "create-channel not accepted on {http_base}: {body}"
    );
    channel_uuid.to_string()
}

// ---------------------------------------------------------------------------
// P1 — Event IDs must not be a cross-tenant oracle (CWE-200/CWE-668).
//
// An event id is content-free, but if community B can REQ an event by id that
// only exists in A, the id becomes an existence+content oracle across the
// tenant boundary. Control: A retrieves it by id.
// ---------------------------------------------------------------------------
#[tokio::test]
#[ignore]
async fn p1_event_id_is_not_a_cross_tenant_oracle() {
    let keys = Keys::generate();
    let marker = format!("adv-iso-p1-{}", uuid::Uuid::new_v4());

    let note = EventBuilder::new(Kind::Custom(1), &marker)
        .sign_with_keys(&keys)
        .unwrap();
    let note_id = note.id;
    let body = submit_event(&http_a(), &keys, &note).await;
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "A must accept the note: {body}"
    );

    // Control: A returns it by id.
    let mut a = BuzzTestClient::connect(&url_a(), &keys)
        .await
        .expect("connect A");
    let sid = sub_id("p1-control");
    a.subscribe(&sid, vec![Filter::new().id(note_id)])
        .await
        .expect("subscribe A");
    let got_a = a
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect A");
    assert!(
        got_a.iter().any(|e| e.id == note_id),
        "CONTROL FAILED: A did not return its own event by id — probe is vacuous"
    );
    a.disconnect().await.expect("disconnect A");

    // Fence: B must not return it — by id, nor by (author, kind) scan.
    let mut b = BuzzTestClient::connect(&url_b(), &keys)
        .await
        .expect("connect B");
    let sid_ids = sub_id("p1-b-ids");
    b.subscribe(&sid_ids, vec![Filter::new().id(note_id)])
        .await
        .expect("subscribe B ids");
    let got_b_ids = b
        .collect_until_eose(&sid_ids, Duration::from_secs(5))
        .await
        .expect("collect B ids");
    assert!(
        got_b_ids.is_empty(),
        "TENANT FENCE BREACH (CWE-668): B returned A's event by id ({} events)",
        got_b_ids.len()
    );

    let sid_scan = sub_id("p1-b-scan");
    b.subscribe(
        &sid_scan,
        vec![Filter::new()
            .kind(Kind::Custom(1))
            .author(keys.public_key())],
    )
    .await
    .expect("subscribe B scan");
    let got_b_scan = b
        .collect_until_eose(&sid_scan, Duration::from_secs(5))
        .await
        .expect("collect B scan");
    assert!(
        !got_b_scan.iter().any(|e| e.id == note_id),
        "TENANT FENCE BREACH (CWE-668): B returned A's event in author+kind scan"
    );
    b.disconnect().await.expect("disconnect B");
}

// ---------------------------------------------------------------------------
// P2 — Live DM (NIP-17 gift wrap) fanout must stay in the publishing
// community (OWASP A01). Same recipient keypair subscribed on BOTH hosts;
// a gift wrap published in A must never arrive at the B subscription.
// Control: A's own subscription receives it.
// ---------------------------------------------------------------------------
#[tokio::test]
#[ignore]
async fn p2_gift_wrap_fanout_stays_in_publishing_community() {
    let sender = Keys::generate();
    let recipient = Keys::generate();

    // B subscription: recipient listens for gift wraps on host B.
    let mut b = BuzzTestClient::connect(&url_b(), &recipient)
        .await
        .expect("connect B");
    let sid_b = sub_id("p2-b");
    b.subscribe(
        &sid_b,
        vec![Filter::new().kind(Kind::Custom(1059)).custom_tag(
            SingleLetterTag::lowercase(Alphabet::P),
            recipient.public_key().to_hex(),
        )],
    )
    .await
    .expect("subscribe B");

    // A subscription (control): recipient listens on host A.
    let mut a = BuzzTestClient::connect(&url_a(), &recipient)
        .await
        .expect("connect A");
    let sid_a = sub_id("p2-a");
    a.subscribe(
        &sid_a,
        vec![Filter::new().kind(Kind::Custom(1059)).custom_tag(
            SingleLetterTag::lowercase(Alphabet::P),
            recipient.public_key().to_hex(),
        )],
    )
    .await
    .expect("subscribe A");

    // Publish the gift wrap in A. Content is opaque to the relay by design
    // (NIP-17); the fanout routing is what is under test. kind:1059 is only
    // accepted via the WebSocket door.
    let wrap = EventBuilder::new(Kind::Custom(1059), "adv-iso-p2-encrypted-payload")
        .tags(vec![
            Tag::parse(["p", &recipient.public_key().to_hex()]).unwrap()
        ])
        .sign_with_keys(&sender)
        .unwrap();
    let wrap_id = wrap.id;
    let mut sender_ws = BuzzTestClient::connect(&url_a(), &sender)
        .await
        .expect("connect sender");
    let ok = sender_ws.send_event(wrap).await.expect("send wrap");
    assert!(
        ok.accepted,
        "A must accept the gift wrap for the control to be meaningful: {}",
        ok.message
    );

    // Control: A's subscription delivers it (live, not just stored). Skip
    // EOSE/OK frames — the live event rides after stored-events drain.
    let mut control_seen = false;
    for _ in 0..8 {
        match a.recv_event(Duration::from_secs(2)).await {
            Ok(buzz_test_client::RelayMessage::Event { event, .. }) => {
                assert_eq!(event.id, wrap_id, "A delivered an unexpected event first");
                control_seen = true;
                break;
            }
            Ok(_) => continue, // EOSE/OK/notice — keep waiting for the event
            Err(_) => break,
        }
    }
    assert!(
        control_seen,
        "CONTROL FAILED: A live subscription never delivered the wrap — probe vacuous"
    );

    // Fence: B's subscription must stay silent for the fanout window.
    let got_b =
        tokio::time::timeout(Duration::from_secs(4), b.recv_event(Duration::from_secs(4))).await;
    if let Ok(Ok(buzz_test_client::RelayMessage::Event { event, .. })) = got_b {
        panic!(
            "TENANT FENCE BREACH (OWASP A01): B live subscription received fanout from A (event id {})",
            event.id
        );
    }

    a.disconnect().await.expect("disconnect A");
    b.disconnect().await.expect("disconnect B");
}

// ---------------------------------------------------------------------------
// P3 — Mention (#p) queries must be tenant-fenced (CWE-668). The feed/
// mentions read path is a stubbed lane in the team's conformance table; this
// independently re-probes the relay-level invariant. Control: A finds it.
// ---------------------------------------------------------------------------
#[tokio::test]
#[ignore]
async fn p3_mention_tag_query_is_tenant_fenced() {
    let author = Keys::generate();
    let mentioned = Keys::generate();

    let note = EventBuilder::new(Kind::Custom(1), "adv-iso-p3 mention body")
        .tags(vec![
            Tag::parse(["p", &mentioned.public_key().to_hex()]).unwrap()
        ])
        .sign_with_keys(&author)
        .unwrap();
    let note_id = note.id;
    let body = submit_event(&http_a(), &author, &note).await;
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "A must accept the mention note: {body}"
    );

    let mention_filter = || {
        Filter::new().kind(Kind::Custom(1)).custom_tag(
            SingleLetterTag::lowercase(Alphabet::P),
            mentioned.public_key().to_hex(),
        )
    };

    // Control: A finds the mention via #p.
    let mut a = BuzzTestClient::connect(&url_a(), &author)
        .await
        .expect("connect A");
    let sid = sub_id("p3-control");
    a.subscribe(&sid, vec![mention_filter()])
        .await
        .expect("sub A");
    let got_a = a
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect A");
    assert!(
        got_a.iter().any(|e| e.id == note_id),
        "CONTROL FAILED: A did not find its own mention — probe vacuous"
    );
    a.disconnect().await.expect("disconnect A");

    // Fence: B must not.
    let mut b = BuzzTestClient::connect(&url_b(), &author)
        .await
        .expect("connect B");
    let sid_b = sub_id("p3-b");
    b.subscribe(&sid_b, vec![mention_filter()])
        .await
        .expect("sub B");
    let got_b = b
        .collect_until_eose(&sid_b, Duration::from_secs(5))
        .await
        .expect("collect B");
    assert!(
        got_b.is_empty(),
        "TENANT FENCE BREACH (CWE-668): B resolved A's mention via #p ({} events)",
        got_b.len()
    );
    b.disconnect().await.expect("disconnect B");
}

// ---------------------------------------------------------------------------
// P4 — Blossom blob fetch across tenants (CWE-200 oracle shape).
//
// The team's conformance table (media row, stubbed) declares shared SHA-256
// blob bytes acceptable but requires metadata/audit boundaries and GENERIC
// errors. What must never happen: B's fetch of an A-uploaded hash returns a
// response that distinguishes "exists in another community" from "does not
// exist anywhere" — an enumeration oracle. We upload in A, then compare B's
// error shape for (a) the A-uploaded hash vs (b) a never-uploaded hash: the
// two must be indistinguishable unless bytes are deliberately shared.
// ---------------------------------------------------------------------------
#[tokio::test]
#[ignore]
async fn p4_blossom_error_shape_is_not_an_existence_oracle() {
    let keys = Keys::generate();
    let payload = format!("adv-iso-p4-blob-{}", uuid::Uuid::new_v4()).into_bytes();
    let sha256 = hex::encode(Sha256::digest(&payload));

    // BUD-02 upload auth: kind 24242, t=upload, fresh + expiration.
    let exp = Timestamp::now().as_secs() + 600;
    let auth_event = EventBuilder::new(Kind::Custom(24242), "adv-iso p4 upload")
        .tags(vec![
            Tag::parse(["t", "upload"]).unwrap(),
            Tag::parse(["x", &sha256]).unwrap(),
            Tag::parse(["expiration", &exp.to_string()]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let auth_header = format!(
        "Nostr {}",
        BASE64.encode(serde_json::to_string(&auth_event).unwrap())
    );

    let client = reqwest::Client::new();
    let up = client
        .put(format!("{}/upload", http_a()))
        .header("Authorization", &auth_header)
        .header("x-sha-256", &sha256)
        .header("Content-Type", "application/octet-stream")
        .body(payload.clone())
        .send()
        .await
        .expect("upload to A");
    let up_status = up.status();
    let up_body: serde_json::Value = up.json().await.unwrap_or_default();
    assert!(
        up_status.is_success(),
        "CONTROL FAILED: upload to A rejected ({up_status}): {up_body}"
    );

    // Control: A serves the blob.
    let get_a = client
        .get(format!("{}/media/{sha256}.bin", http_a()))
        .send()
        .await
        .expect("GET from A");
    let a_status = get_a.status();
    assert!(
        a_status.is_success(),
        "CONTROL FAILED: A does not serve its own upload ({a_status}) — probe vacuous"
    );

    // Fence/oracle check: B's response to the A-uploaded hash vs a
    // never-uploaded hash must be INDISTINGUISHABLE (both 404), or the blob
    // must be deliberately shared (both 200 with correct bytes only for the
    // real hash — never-uploaded must 404). Any 4xx/2xx asymmetry between
    // "exists in A" and "exists nowhere" is an existence oracle (CWE-200).
    let never_sha = hex::encode(Sha256::digest(b"adv-iso-p4-never-existed"));
    let get_b_real = client
        .get(format!("{}/media/{sha256}.bin", http_b()))
        .send()
        .await
        .expect("GET real from B");
    let b_real_status = get_b_real.status();
    let b_real_body = get_b_real.text().await.unwrap_or_default();
    let get_b_never = client
        .get(format!("{}/media/{never_sha}.bin", http_b()))
        .send()
        .await
        .expect("GET never from B");
    let b_never_status = get_b_never.status();
    let b_never_body = get_b_never.text().await.unwrap_or_default();

    if b_real_status.is_success() {
        // Deliberately shared bytes: only the real hash may resolve.
        eprintln!(
            "P4 variant: SHARED BYTES — B served A's blob ({b_real_status}); never-uploaded hash: {b_never_status}"
        );
        assert!(
            b_never_status.as_u16() == 404,
            "shared-bytes design broken: never-uploaded hash returned {b_never_status} on B"
        );
    } else {
        eprintln!(
            "P4 variant: FENCED — B 404s A's hash and never-uploaded hash identically ({b_real_status})"
        );
        assert_eq!(
            b_real_status.as_u16(),
            b_never_status.as_u16(),
            "EXISTENCE ORACLE (CWE-200): B distinguishes A-uploaded hash ({b_real_status}: {b_real_body}) from never-uploaded ({b_never_status}: {b_never_body})"
        );
        assert_eq!(
            b_real_body, b_never_body,
            "EXISTENCE ORACLE (CWE-200): B's error bodies differ ({b_real_body} vs {b_never_body})"
        );
    }
}

// ---------------------------------------------------------------------------
// P7 — Workflow webhook door must fail closed cross-tenant (OWASP A01).
//
// A's workflow UUID + valid secret, fired at B's host, must meet the same
// generic 404 as a nonexistent workflow — the door derives the tenant from
// the Host, not from the UUID. Control arms: A + wrong secret → 401;
// A + correct secret → accepted.
// ---------------------------------------------------------------------------
#[tokio::test]
#[ignore]
async fn p7_workflow_webhook_fails_closed_cross_tenant() {
    let keys = Keys::generate();
    let channel_id = create_channel(&http_a(), &keys).await;

    // Define a webhook-trigger workflow in A (kind:30620, d = workflow UUID).
    let workflow_id = uuid::Uuid::new_v4().to_string();
    let yaml = "name: adv-iso-p7\ndescription: webhook tenant fence probe\ntrigger:\n  on: webhook\nsteps:\n - id: s1\n   name: Notify\n   action: send_message\n   text: \"p7\"\n";
    let def = EventBuilder::new(Kind::Custom(30620), yaml)
        .tags(vec![
            Tag::parse(["d", &workflow_id]).unwrap(),
            Tag::parse(["h", &channel_id]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let body = submit_event(&http_a(), &keys, &def).await;
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "A must accept the workflow def: {body}"
    );
    let msg = body["message"].as_str().unwrap_or_default();
    let resp_json: serde_json::Value = serde_json::from_str(
        msg.strip_prefix("response:")
            .unwrap_or_else(|| panic!("def OK message missing response prefix: {msg}")),
    )
    .expect("parse def response");
    let secret = resp_json["webhook_secret"]
        .as_str()
        .expect("webhook workflow must return a secret")
        .to_string();

    let client = reqwest::Client::new();

    // Control arm 1: A + wrong secret → 401.
    let wrong = client
        .post(format!("{}/hooks/{workflow_id}", http_a()))
        .header("x-webhook-secret", "adv-iso-wrong-secret")
        .body("{}")
        .send()
        .await
        .expect("fire wrong secret at A");
    assert_eq!(
        wrong.status().as_u16(),
        401,
        "CONTROL FAILED: A accepted a wrong webhook secret ({})",
        wrong.status()
    );

    // Fence arm: B + correct UUID + correct secret → generic 404, identical
    // to B's 404 for a never-existing workflow UUID (no existence oracle).
    let never_id = uuid::Uuid::new_v4();
    let cross = client
        .post(format!("{}/hooks/{workflow_id}", http_b()))
        .header("x-webhook-secret", &secret)
        .body("{}")
        .send()
        .await
        .expect("fire at B");
    let cross_status = cross.status().as_u16();
    let cross_body = cross.text().await.unwrap_or_default();
    let never = client
        .post(format!("{}/hooks/{never_id}", http_b()))
        .header("x-webhook-secret", &secret)
        .body("{}")
        .send()
        .await
        .expect("fire never-id at B");
    let never_status = never.status().as_u16();
    let never_body = never.text().await.unwrap_or_default();

    assert_eq!(
        cross_status, 404,
        "TENANT FENCE BREACH (OWASP A01): B's webhook door answered A's workflow UUID with {cross_status}: {cross_body}"
    );
    assert_eq!(
        cross_status, never_status,
        "EXISTENCE ORACLE (CWE-200): B distinguishes A's workflow ({cross_status}) from never-existing ({never_status})"
    );
    assert_eq!(
        cross_body, never_body,
        "EXISTENCE ORACLE (CWE-200): B's 404 bodies differ ({cross_body} vs {never_body})"
    );

    // Control arm 2: A + correct secret → accepted (proves the UUID+secret
    // pair was real, so the B-side 404 is the fence, not a dead workflow).
    let good = client
        .post(format!("{}/hooks/{workflow_id}", http_a()))
        .header("x-webhook-secret", &secret)
        .body("{}")
        .send()
        .await
        .expect("fire correct secret at A");
    assert!(
        good.status().is_success(),
        "CONTROL FAILED: A rejected its own webhook with the correct secret: {}",
        good.status()
    );
}

// ---------------------------------------------------------------------------
// P9 — Same-coordinate addressable events coexist, each tenant-scoped
// (CWE-668). The buzz-db lane in the team's conformance table ("same id/d-tag
// in A and B both retrievable, each scoped; no cross-fetch") is stubbed; this
// probes it independently: identical (kind, author, d-tag) in A and B with
// different content — each host must serve ONLY its own version.
// ---------------------------------------------------------------------------
#[tokio::test]
#[ignore]
async fn p9_same_coordinate_addressable_events_stay_scoped() {
    let keys = Keys::generate();
    let d_tag = format!("adv-iso-p9-{}", uuid::Uuid::new_v4().simple());

    let mk = |content: &str| {
        EventBuilder::new(Kind::Custom(30023), content)
            .tags(vec![Tag::parse(["d", &d_tag]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap()
    };
    let ev_a = mk("adv-iso-p9-CONTENT-A");
    let ev_b = mk("adv-iso-p9-CONTENT-B");
    let id_a = ev_a.id;
    let id_b = ev_b.id;

    let body_a = submit_event(&http_a(), &keys, &ev_a).await;
    assert!(
        body_a["accepted"].as_bool().unwrap_or(false),
        "A must accept its version: {body_a}"
    );
    let body_b = submit_event(&http_b(), &keys, &ev_b).await;
    assert!(
        body_b["accepted"].as_bool().unwrap_or(false),
        "B must accept its version: {body_b}"
    );

    let coord_filter = || {
        Filter::new()
            .kind(Kind::Custom(30023))
            .author(keys.public_key())
            .custom_tag(SingleLetterTag::lowercase(Alphabet::D), d_tag.as_str())
    };

    let mut a = BuzzTestClient::connect(&url_a(), &keys)
        .await
        .expect("connect A");
    let sid_a = sub_id("p9-a");
    a.subscribe(&sid_a, vec![coord_filter()])
        .await
        .expect("sub A");
    let got_a = a
        .collect_until_eose(&sid_a, Duration::from_secs(5))
        .await
        .expect("collect A");
    assert!(
        got_a.iter().any(|e| e.id == id_a),
        "CONTROL FAILED: A does not serve its own version"
    );
    assert!(
        !got_a
            .iter()
            .any(|e| e.id == id_b || e.content.contains("CONTENT-B")),
        "TENANT FENCE BREACH (CWE-668): A serves B's version of the same coordinate"
    );
    a.disconnect().await.expect("disconnect A");

    let mut b = BuzzTestClient::connect(&url_b(), &keys)
        .await
        .expect("connect B");
    let sid_b = sub_id("p9-b");
    b.subscribe(&sid_b, vec![coord_filter()])
        .await
        .expect("sub B");
    let got_b = b
        .collect_until_eose(&sid_b, Duration::from_secs(5))
        .await
        .expect("collect B");
    assert!(
        got_b.iter().any(|e| e.id == id_b),
        "CONTROL FAILED: B does not serve its own version"
    );
    assert!(
        !got_b
            .iter()
            .any(|e| e.id == id_a || e.content.contains("CONTENT-A")),
        "TENANT FENCE BREACH (CWE-668): B serves A's version of the same coordinate"
    );
    b.disconnect().await.expect("disconnect B");
}

// ---------------------------------------------------------------------------
// P10 — Ingest door robustness battery (OWASP A05/A10; CWE-20 improper input
// validation). Malformed and hostile inputs at POST /events must be rejected
// with 4xx — never 5xx — and the relay must remain healthy and keep accepting
// valid traffic afterwards. A panic or 500 here is a remote DoS primitive.
// ---------------------------------------------------------------------------
#[tokio::test]
#[ignore]
async fn p10_ingest_door_never_5xx_on_hostile_input() {
    let keys = Keys::generate();
    let client = reqwest::Client::new();
    let events_url = format!("{}/events", http_a());

    // A known-valid event for the liveness arms.
    let valid = EventBuilder::new(Kind::Custom(1), "adv-iso-p10-valid")
        .sign_with_keys(&keys)
        .unwrap();
    let valid_json = serde_json::to_string(&valid).unwrap();

    // (body, descriptor) — every one must be rejected 4xx, never 5xx.
    let huge_content = "x".repeat(2 * 1024 * 1024);
    let huge_ev = EventBuilder::new(Kind::Custom(1), huge_content)
        .sign_with_keys(&keys)
        .unwrap();
    let mut bad_sig = serde_json::to_value(&valid).unwrap();
    bad_sig["sig"] = serde_json::Value::String("00".repeat(64));
    let mut bad_id = serde_json::to_value(&valid).unwrap();
    bad_id["id"] = serde_json::Value::String("11".repeat(32));
    let mut bad_kind = serde_json::to_value(&valid).unwrap();
    bad_kind["kind"] = serde_json::json!(4_000_000_000u64); // exceeds u16/u32 kind space
    let mut bad_tags = serde_json::to_value(&valid).unwrap();
    bad_tags["tags"] = serde_json::json!({"not": "an array of arrays"});
    let mut bad_pubkey = serde_json::to_value(&valid).unwrap();
    bad_pubkey["pubkey"] = serde_json::Value::String("zz".repeat(32));
    let mut future_ts = serde_json::to_value(&valid).unwrap();
    future_ts["created_at"] = serde_json::json!(4_102_444_800u64); // year 2100

    let cases: Vec<(String, &str)> = vec![
        ("{not json".to_string(), "truncated JSON"),
        ("[]".to_string(), "empty array, not an event object"),
        ("{}".to_string(), "empty object"),
        (
            serde_json::to_string(&bad_sig).unwrap(),
            "corrupted signature",
        ),
        (
            serde_json::to_string(&bad_id).unwrap(),
            "id/content mismatch",
        ),
        (
            serde_json::to_string(&bad_kind).unwrap(),
            "kind out of range",
        ),
        (
            serde_json::to_string(&bad_tags).unwrap(),
            "tags not array-of-arrays",
        ),
        (
            serde_json::to_string(&bad_pubkey).unwrap(),
            "pubkey not hex",
        ),
        (
            serde_json::to_string(&future_ts).unwrap(),
            "far-future created_at",
        ),
        (serde_json::to_string(&huge_ev).unwrap(), "2MB content"),
    ];

    let mut rejects = 0u32;
    for (body, desc) in &cases {
        let resp = client
            .post(&events_url)
            .header("X-Pubkey", keys.public_key().to_hex())
            .header("Content-Type", "application/json")
            .body(body.clone())
            .send()
            .await
            .unwrap_or_else(|e| panic!("door unreachable on case '{desc}': {e}"));
        let status = resp.status().as_u16();
        assert!(
            (400..500).contains(&status),
            "case '{desc}': expected 4xx rejection, got {status} — a 5xx here is a DoS primitive (CWE-20)"
        );
        rejects += 1;
    }
    assert_eq!(
        rejects as usize,
        cases.len(),
        "every case must be evaluated"
    );

    // Liveness: the relay survived the barrage and still accepts valid events.
    let resp = client
        .post(&events_url)
        .header("X-Pubkey", keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(valid_json)
        .send()
        .await
        .expect("liveness POST");
    let body: serde_json::Value = resp.json().await.expect("liveness parse");
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "relay failed to accept a valid event after the hostile barrage: {body}"
    );
}

// ---------------------------------------------------------------------------
// P8 — POST /count bridge must be tenant-fenced (CWE-668). Control: A counts
// the planted note; fence: B counts zero for the same filter.
// ---------------------------------------------------------------------------
#[tokio::test]
#[ignore]
async fn p8_count_bridge_is_tenant_fenced() {
    let keys = Keys::generate();
    let marker = format!("adv-iso-p8-{}", uuid::Uuid::new_v4());
    let note = EventBuilder::new(Kind::Custom(1), &marker)
        .sign_with_keys(&keys)
        .unwrap();
    let body = submit_event(&http_a(), &keys, &note).await;
    assert!(
        body["accepted"].as_bool().unwrap_or(false),
        "A must accept the note: {body}"
    );

    let count_req = serde_json::json!([{
        "kinds": [1],
        "authors": [keys.public_key().to_hex()],
    }]);
    let client = reqwest::Client::new();

    let count_a: serde_json::Value = client
        .post(format!("{}/count", http_a()))
        .header("X-Pubkey", keys.public_key().to_hex())
        .json(&count_req)
        .send()
        .await
        .expect("count A")
        .json()
        .await
        .expect("parse count A");
    let n_a = count_a["count"].as_u64().unwrap_or(0);
    assert!(n_a >= 1, "CONTROL FAILED: A counted {n_a} of its own note");

    let count_b: serde_json::Value = client
        .post(format!("{}/count", http_b()))
        .header("X-Pubkey", keys.public_key().to_hex())
        .json(&count_req)
        .send()
        .await
        .expect("count B")
        .json()
        .await
        .expect("parse count B");
    let n_b = count_b["count"].as_u64().unwrap_or(0);
    assert_eq!(
        n_b, 0,
        "TENANT FENCE BREACH (CWE-668): B counted {n_b} events authored in A"
    );
}
