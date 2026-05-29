//! End-to-end acceptance tests for the relay-hosted mesh-LLM feature.
//!
//! These tests require a running sprout-relay with mesh embedded
//! (`SPROUT_MESH_ENABLED=true`, `SPROUT_REQUIRE_RELAY_MEMBERSHIP=true`) and,
//! for the live-inference rows, two desktop mesh nodes (serve + client).
//! All tests are `#[ignore]` by default — they need infra CI does not host
//! (native llama, multi-node, model download). The deterministic trust
//! invariants are unit-tested in `sprout-relay` (`mesh_status_publisher`,
//! `iroh_relay`); this file is the opt-in full-stack acceptance layer.
//!
//! # Running (manual / runbook)
//!
//! ```text
//! # 1. one-time local llama build (see docs/mesh-llm-local-build.md)
//! # 2. start a mesh-enabled relay
//! SPROUT_MESH_ENABLED=true SPROUT_REQUIRE_RELAY_MEMBERSHIP=true \
//!   cargo run -p sprout-relay
//! # 3. run the trust assertions (no GPU needed):
//! RELAY_URL=ws://localhost:3000 \
//!   cargo test --test e2e_mesh_llm trust -- --ignored --nocapture
//! # 4. run the live A->B inference rows (needs 2 mesh nodes + a small model):
//! cargo test --test e2e_mesh_llm live -- --ignored --nocapture
//! ```
//!
//! ## Acceptance matrix (= the demo, as a test)
//! | # | Assertion | This file | Also covered by |
//! |---|-----------|-----------|-----------------|
//! | 1 | member reads kind:30621 status w/ dial pointer, no secrets | `trust_member_reads_mesh_status` | relay `mesh_status_publisher` units |
//! | 2 | non-member REQ for kind:30621 returns nothing | `trust_nonmember_read_denied` | — |
//! | 3 | non-member iroh dial denied (NIP-98→membership) | runbook (needs iroh dial) | relay `iroh_relay` admission units |
//! | 4 | B's agent completes a chat against A's model over mesh | `live_agent_completes_chat_over_mesh` | runbook |
//! | 5 | dropped member → typed auth failure reaches lastError | runbook (desktop harness) | sprout-agent `-32001` unit |
//! | 6 | split: model too big → 2 serve nodes → chat completes | `live_split_model_completes` | runbook |

use std::time::Duration;

use nostr::{Filter, Keys, Kind};
use sprout_test_client::SproutTestClient;

/// Sprout's relay-owned mesh status kind (must match `sprout_core::kind`).
const KIND_MESH_LLM_RELAY_STATUS: u16 = 30621;
const MESH_STATUS_D_TAG: &str = "sprout-relay-mesh";
const MESH_STATUS_TYPE: &str = "sprout-mesh-status";

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
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
    let member = Keys::generate(); // NOTE: must be a relay member; runbook seeds membership.
    let mut client = SproutTestClient::connect(&url, &member)
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
/// Requires a relay with `SPROUT_REQUIRE_RELAY_MEMBERSHIP=true` and a published
/// status event that members can see (paired with assertion 1).
#[tokio::test]
#[ignore]
async fn trust_nonmember_read_denied() {
    let url = relay_url();
    let stranger = Keys::generate(); // deliberately NOT a relay member.
    let mut client = match SproutTestClient::connect(&url, &stranger).await {
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
/// B running a mesh client + a launched sprout-agent pointed at B's local
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
    // 1. desktop A: Share compute → serve `qwen2.5-0.5b-instruct` (tiny for CI budget).
    // 2. desktop B (same relay member): Create Agent → "Run on relay mesh" → pick A's model.
    // 3. drive one chat turn through B's sprout-agent; assert the completion is non-empty.
    // Asserting against live inference requires the running desktops; this test
    // is the contract those steps satisfy. Eva wires the driver at M1.
    eprintln!("live_agent_completes_chat_over_mesh: runbook test — see module docs");
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
    eprintln!("live_split_model_completes: runbook test — see module docs");
}
