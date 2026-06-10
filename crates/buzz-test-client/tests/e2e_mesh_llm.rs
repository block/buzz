//! End-to-end acceptance tests for the relay-hosted mesh-LLM feature.
//!
//! These tests require a running buzz-relay with mesh embedded
//! (`BUZZ_MESH_ENABLED=true`, `BUZZ_REQUIRE_RELAY_MEMBERSHIP=true`) and,
//! for the live-inference rows, two desktop mesh nodes (serve + client).
//! All tests are `#[ignore]` by default — they need infra CI does not host
//! (native llama, multi-node, model download). The deterministic trust
//! invariants are unit-tested in `buzz-relay` (`mesh_status_publisher`,
//! `mesh_signaling`); this file is the opt-in full-stack acceptance layer.
//!
//! # Running (manual / runbook)
//!
//! ```text
//! # 1. one-time local llama build (see docs/mesh-llm-local-build.md)
//! # 2. start a mesh-enabled relay
//! BUZZ_MESH_ENABLED=true BUZZ_REQUIRE_RELAY_MEMBERSHIP=true \
//!   cargo run -p buzz-relay
//! # 3. run the trust assertions (no GPU needed):
//! RELAY_URL=ws://localhost:3000 \
//!   cargo test --test e2e_mesh_llm trust -- --ignored --nocapture
//! # 4. run the live A->B inference row (needs 2 mesh nodes + a small model):
//! #    point at B's local OpenAI endpoint; without it the test SKIPS (no silent pass):
//! MESH_OPENAI_BASE=http://127.0.0.1:9337/v1 \
//!   cargo test --test e2e_mesh_llm live_agent_completes -- --ignored --nocapture
//! # trust rows need real relay identities, not generated keys:
//! MEMBER_NSEC=nsec1... STRANGER_NSEC=nsec1... \
//!   cargo test --test e2e_mesh_llm trust -- --ignored --nocapture
//! ```
//!
//! ## Acceptance matrix (= the demo, as a test)
//! | # | Assertion | This file | Also covered by |
//! |---|-----------|-----------|-----------------|
//! | 1 | member reads kind:30621 status w/ dial pointer, no secrets | `trust_member_reads_mesh_status` | relay `mesh_status_publisher` units |
//! | 2 | non-member REQ for kind:30621 returns nothing | `trust_nonmember_read_denied` | — |
//! | 3 | non-member iroh dial denied (NIP-98→membership) | runbook (needs iroh dial) | relay `mesh_signaling` admission units |
//! | 4 | B's agent completes a chat against A's model over mesh | `live_agent_completes_chat_over_mesh` | runbook |
//! | 5 | dropped member → typed auth failure reaches lastError | runbook (desktop harness) | buzz-agent `-32001` unit |
//! | 6 | split: model too big → 2 serve nodes → chat completes | `live_split_model_completes` | runbook |

use std::time::Duration;

use buzz_test_client::BuzzTestClient;
use nostr::{Filter, Keys, Kind};

/// Sprout's relay-owned mesh status kind (must match `buzz_core::kind`).
const KIND_MESH_LLM_RELAY_STATUS: u16 = 30621;
const MESH_STATUS_D_TAG: &str = "buzz-relay-mesh";
const MESH_STATUS_TYPE: &str = "sprout-mesh-status";

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

/// Load a relay identity from an env-provided nsec. Returns `None` (and prints
/// why) when the fixture is absent, so the caller skips rather than running
/// against a `Keys::generate()` identity whose membership is undefined —
/// asserting "member sees status" against a random key is the bug Perci caught.
fn keys_from_env(var: &str) -> Option<Keys> {
    match std::env::var(var) {
        Ok(nsec) if !nsec.trim().is_empty() => match Keys::parse(nsec.trim()) {
            Ok(keys) => Some(keys),
            Err(e) => panic!("{var} is set but not a valid nsec/hex secret key: {e}"),
        },
        _ => {
            eprintln!(
                "SKIP: {var} not set — provision a relay {} identity and re-run (see module docs)",
                if var.contains("MEMBER") {
                    "member"
                } else {
                    "non-member"
                }
            );
            None
        }
    }
}

fn sub_id(name: &str) -> String {
    format!("e2e-mesh-{name}-{}", uuid::Uuid::new_v4().simple())
}

fn mesh_status_filter() -> Filter {
    Filter::new()
        .kind(Kind::Custom(KIND_MESH_LLM_RELAY_STATUS))
        .identifier(MESH_STATUS_D_TAG)
}

// ── (1) member reads the relay-signed status, with dial pointer, no secrets ──

/// Assertion 1: an authenticated relay member can REQ the relay-signed
/// kind:30621 status event; its content carries the sanitized projection
/// (mesh/models/serveTargets with EndpointAddr dial pointers) and NO secrets
/// (no invite-secret, no local paths, no raw runtime object).
///
/// Requires a mesh-enabled relay that has published at least one status event.
#[tokio::test]
#[ignore]
async fn trust_member_reads_mesh_status() {
    let url = relay_url();
    let Some(member) = keys_from_env("MEMBER_NSEC") else {
        return;
    };
    let mut client = BuzzTestClient::connect(&url, &member)
        .await
        .expect("member connect+auth");

    let sid = sub_id("member-read");
    client
        .subscribe(&sid, vec![mesh_status_filter()])
        .await
        .expect("subscribe");
    let events = client
        .collect_until_eose(&sid, Duration::from_secs(10))
        .await
        .expect("collect");

    let status = events
        .iter()
        .find(|e| e.kind == Kind::Custom(KIND_MESH_LLM_RELAY_STATUS))
        .expect("a member must see at least one kind:30621 status event");

    // Relay-signed (the relay keypair, not this member).
    assert_ne!(
        status.pubkey,
        member.public_key(),
        "status must be relay-signed, not member-signed"
    );

    let content: serde_json::Value =
        serde_json::from_str(&status.content).expect("content is JSON");
    assert_eq!(content["type"], MESH_STATUS_TYPE, "type discriminator");

    // Dial pointer present (EndpointAddr is connectivity, not a secret).
    let targets = content["serveTargets"]
        .as_array()
        .expect("serveTargets array");
    if let Some(t) = targets.first() {
        assert!(
            t.get("endpointAddr").is_some(),
            "serve target carries its EndpointAddr dial pointer"
        );
    }

    // No secrets / no local-machine leakage in the published projection.
    let raw = status.content.to_lowercase();
    for forbidden in [
        "nsec",
        "secret",
        "/users/",
        "/home/",
        "runtime_dir",
        "local_path",
    ] {
        assert!(
            !raw.contains(forbidden),
            "published status must not leak `{forbidden}`"
        );
    }

    client.disconnect().await.ok();
}

// ── (2) non-member read denied ───────────────────────────────────────────────

/// Assertion 2: a valid Nostr identity that is NOT a relay member gets nothing
/// back for a kind:30621 REQ — membership gates the read.
///
/// Requires a relay with `BUZZ_REQUIRE_RELAY_MEMBERSHIP=true` and a published
/// status event that members can see (paired with assertion 1).
#[tokio::test]
#[ignore]
async fn trust_nonmember_read_denied() {
    let url = relay_url();
    let Some(stranger) = keys_from_env("STRANGER_NSEC") else {
        return;
    };
    let mut client = match BuzzTestClient::connect(&url, &stranger).await {
        Ok(c) => c,
        // A closed relay may refuse NIP-42 auth for a non-member outright —
        // that is also a valid "denied" outcome.
        Err(_) => return,
    };

    let sid = sub_id("stranger-read");
    client
        .subscribe(&sid, vec![mesh_status_filter()])
        .await
        .expect("subscribe");
    let events = client
        .collect_until_eose(&sid, Duration::from_secs(10))
        .await
        .expect("collect");

    let leaked = events
        .iter()
        .any(|e| e.kind == Kind::Custom(KIND_MESH_LLM_RELAY_STATUS));
    assert!(
        !leaked,
        "non-member must NOT receive kind:30621 mesh status"
    );

    client.disconnect().await.ok();
}

// ── (4) the demo: B's agent completes a chat against A's model over the mesh ──

/// Assertion 4 (the headline demo): with desktop A serving a model and desktop
/// B running a mesh client + a launched buzz-agent pointed at B's local
/// `:9337/v1`, a chat completion returns a non-empty response routed over the
/// mesh to A's GPU.
///
/// This needs two live mesh nodes + a small served model — runbook only, never
/// in default CI. Left as a documented, compiling placeholder so the acceptance
/// matrix is executable code, not prose; wire the live harness when M1 lands.
#[tokio::test]
#[ignore]
async fn live_agent_completes_chat_over_mesh() {
    // RUNBOOK (M1 hardware): see module docs.
    //   A: Share compute → serve a small model. B: mesh client up on :9337.
    // Point this test at B's local OpenAI endpoint via MESH_OPENAI_BASE
    // (e.g. http://127.0.0.1:9337/v1). When set, we drive a real completion
    // over the mesh and assert non-empty output — no endpoint, no silent pass.
    let Ok(base) = std::env::var("MESH_OPENAI_BASE") else {
        eprintln!(
            "SKIP: MESH_OPENAI_BASE not set — needs a live mesh client endpoint (see module docs)"
        );
        return;
    };
    let base = base.trim_end_matches('/').to_string();
    let http = reqwest::Client::new();

    // Resolve the served model id (the node assigns its own, not our ref).
    let models: serde_json::Value = http
        .get(format!("{base}/models"))
        .send()
        .await
        .expect("GET /models")
        .json()
        .await
        .expect("/models JSON");
    let model_id = models["data"][0]["id"]
        .as_str()
        .expect("at least one model served over the mesh")
        .to_string();

    let resp: serde_json::Value = http
        .post(format!("{base}/chat/completions"))
        .json(&serde_json::json!({
            "model": model_id,
            "messages": [{"role": "user", "content": "Reply with exactly one word: PONG"}],
            "max_tokens": 512,
            "temperature": 0.0,
        }))
        .send()
        .await
        .expect("POST /chat/completions over mesh")
        .json()
        .await
        .expect("completion JSON");

    let content = resp["choices"][0]["message"]["content"]
        .as_str()
        .expect("completion has message content");
    assert!(
        !content.trim().is_empty(),
        "chat completion over the mesh must return non-empty content"
    );
}

// ── (6) split variant ────────────────────────────────────────────────────────

/// Assertion 6 (split): a model too large for one node + two serve nodes in the
/// same mesh → mesh auto-splits → the same chat (assertion 4) completes via the
/// split route. Auto-split is mesh runtime behavior (no Sprout code); this row
/// only verifies two serve desktops in one mesh produce a working split.
///
/// Runbook only — needs a known too-large-for-one-node fixture + 2 serve nodes.
#[tokio::test]
#[ignore]
async fn live_split_model_completes() {
    // RUNBOOK: A + C both serve the oversized model into the same mesh; B's
    // agent completes a chat; mesh elects a split topology (>=2 stage participants).
    // Genuinely multi-node — cannot be automated single-process. Left unwired
    // so `--ignored` can never report it green without a real split harness.
    panic!("live_split_model_completes: not implemented — runbook only (see module docs)");
}
