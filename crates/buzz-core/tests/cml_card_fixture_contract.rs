//! Pins the desktop CML card fixture to the live Rust serializer.
//!
//! The desktop TypeScript layer
//! (`desktop/src/features/workstream/cmlCard.test.mjs`) asserts against a
//! checked-in JSON fixture rather than a hand-written guess at the wire shape.
//! That buys real-bytes fidelity, but it introduces a drift hazard: if the
//! Rust [`WorkstreamCard`] shape ever changes, the fixture silently becomes a
//! lie and the TypeScript tests keep passing against a stale contract.
//!
//! This test fails the Rust suite the moment the two sides diverge, so the
//! wire contract stays single-sourced from the serializer that actually
//! produces it.

use buzz_core::cml::{parse_cml, CmlTask};
use buzz_core::cml_view::project_workstream_card;

const WORKER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const PLANNER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BASE_SHA: &str = "1111111111111111111111111111111111111111";
const HEAD_SHA: &str = "abcdef0123456789abcdef0123456789abcdef01";
const UPDATED_AT: u64 = 1_787_673_000;
const TTL: u64 = 180;

/// Path of the fixture consumed by the desktop test suite.
const FIXTURE_REL: &str = "../../desktop/src/features/workstream/cmlCardFixtures.json";

/// A claimed task whose snapshot is internally consistent at `UPDATED_AT`.
///
/// `heartbeat_age` is the heartbeat age *at snapshot time*, so the stored
/// `presence` stays schema-valid; `lease_expires_at` is absolute.
fn claimed_task(
    heartbeat_age: Option<u64>,
    lease_expires_at: u64,
    head_sha: Option<&str>,
) -> CmlTask {
    let (heartbeat, presence) = match heartbeat_age {
        None => (serde_json::Value::Null, "offline"),
        Some(age) => {
            let presence = if age <= TTL {
                "online"
            } else if age <= TTL * 2 {
                "stale"
            } else {
                "offline"
            };
            (serde_json::json!(UPDATED_AT - age), presence)
        }
    };
    let value = serde_json::json!({
        "acceptance": [{"id":"A1","text":"Card reflects observation time","verified":false}],
        "blockers": [],
        "evidence": [],
        "git": {
            "base_sha": BASE_SHA,
            "branch": "feat/cml-view",
            "head_sha": head_sha,
            "repo": "block/buzz",
            "worktree_alias": "buzz-s4"
        },
        "id": "cdd4722d-7481-4d01-9c0a-423b4454c179",
        "lease": {
            "id": "lease-1",
            "holder": WORKER,
            "issued_at": UPDATED_AT - 600,
            "expires_at": lease_expires_at
        },
        "objective": "Board shows liveness recomputed at view time",
        "priority": "P1",
        "protocol": "buzz-cml",
        "review": {"max_rounds":3,"round":0},
        "roles": {"fixer":null,"planner":PLANNER,"reviewer":null,"worker":WORKER},
        "runtime": {
            "host_id": "h_0123456789abcdef",
            "last_heartbeat_at": heartbeat,
            "presence": presence,
            "ttl_seconds": TTL
        },
        "status": "claimed",
        "title": "CML view projection",
        "updated_at": UPDATED_AT,
        "version": 1
    });
    parse_cml(&value.to_string()).expect("fixture must be valid CML")
}

/// The scenario set mirrors the S4 acceptance list, including the case that
/// matters most: a fresh heartbeat whose lease has already expired is online
/// but is **not** a live claim.
fn scenarios() -> Vec<(&'static str, CmlTask, u64)> {
    vec![
        (
            "online_live_claim",
            claimed_task(Some(60), UPDATED_AT + 600, Some(HEAD_SHA)),
            UPDATED_AT,
        ),
        (
            "stale_not_live",
            claimed_task(Some(60), UPDATED_AT + 600, Some(HEAD_SHA)),
            UPDATED_AT + 180,
        ),
        (
            "offline_two_ttl",
            claimed_task(Some(60), UPDATED_AT + 6000, Some(HEAD_SHA)),
            UPDATED_AT + 400,
        ),
        (
            "no_heartbeat_offline",
            claimed_task(None, UPDATED_AT + 600, Some(HEAD_SHA)),
            UPDATED_AT,
        ),
        (
            "fresh_heartbeat_expired_lease",
            claimed_task(Some(60), UPDATED_AT - 1, Some(HEAD_SHA)),
            UPDATED_AT,
        ),
        (
            "missing_head_sha",
            claimed_task(Some(60), UPDATED_AT + 600, None),
            UPDATED_AT,
        ),
    ]
}

/// Serialize every scenario exactly as the desktop fixture stores it.
fn generate() -> serde_json::Value {
    let mut out = serde_json::Map::new();
    for (name, task, observed_at) in scenarios() {
        let card = project_workstream_card(&task, observed_at);
        let mut entry = serde_json::Map::new();
        entry.insert("observed_at".into(), serde_json::json!(observed_at));
        entry.insert(
            "card".into(),
            serde_json::to_value(&card).expect("serialize card"),
        );
        out.insert(name.into(), serde_json::Value::Object(entry));
    }
    serde_json::Value::Object(out)
}

fn fixture_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_REL)
}

#[test]
fn desktop_fixture_matches_the_live_serializer() {
    let path = fixture_path();
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "desktop CML card fixture missing at {}: {error}",
            path.display()
        )
    });
    let on_disk: serde_json::Value =
        serde_json::from_str(&raw).expect("fixture must be valid JSON");

    assert_eq!(
        on_disk,
        generate(),
        "desktop fixture has drifted from the Rust WorkstreamCard serializer; \
         regenerate {} with: cargo test -p buzz-core --test cml_card_fixture_contract \
         -- --ignored --nocapture",
        path.display()
    );
}

/// Regeneration helper. Ignored by default so it never rewrites expectations
/// as a side effect of a normal test run.
#[test]
#[ignore = "regeneration helper; run explicitly with --ignored"]
fn emit_desktop_fixture() {
    let rendered = serde_json::to_string_pretty(&generate()).expect("pretty");
    println!("---GOLDEN-START---");
    println!("{rendered}");
    println!("---GOLDEN-END---");
}
