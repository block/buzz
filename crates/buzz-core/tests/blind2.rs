//! Blind black-box verification of `cml_view::project_workstream_card`.
//!
//! Fixtures are built as strict CML v1 JSON and parsed with `parse_cml`, so
//! every input satisfies the stored-schema invariants (ttl 180, stored
//! presence consistent at `updated_at`, lease rules). The card is then judged
//! only through the public projection API at a caller-chosen `observed_at`.

use buzz_core::cml::{parse_cml, CmlTask, Presence};
use buzz_core::cml_view::project_workstream_card;
use serde_json::{json, Value};

const PLANNER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const WORKER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const BASE_SHA: &str = "1234567890123456789012345678901234567890";
const HEAD_SHA: &str = "abcdef0123456789abcdef0123456789abcdef01";

/// Base strict-valid document: status `working`, worker-held lease,
/// heartbeat at T0, updated_at = T0 + 100 so stored presence is `online`.
fn working_task_json() -> Value {
    let t0: u64 = 1_700_000_000;
    json!({
        "acceptance": [{ "id": "A1", "text": "Criterion", "verified": false }],
        "blockers": [],
        "evidence": [],
        "git": {
            "base_sha": BASE_SHA,
            "branch": "feat/blind",
            "head_sha": null,
            "repo": "block/buzz",
            "worktree_alias": "blind-wt"
        },
        "id": "cdd4722d-7481-4d01-9c0a-423b4454c179",
        "lease": {
            "id": "lease-1",
            "holder": WORKER,
            "issued_at": t0 - 100,
            "expires_at": t0 + 10_000
        },
        "objective": "One testable outcome",
        "priority": "P1",
        "protocol": "buzz-cml",
        "review": { "max_rounds": 3, "round": 0 },
        "roles": {
            "fixer": null,
            "planner": PLANNER,
            "reviewer": null,
            "worker": WORKER
        },
        "runtime": {
            "host_id": null,
            "last_heartbeat_at": t0,
            "presence": "online",
            "ttl_seconds": 180
        },
        "status": "working",
        "title": "Blind task",
        "updated_at": t0 + 100,
        "version": 1
    })
}

fn parse(value: &Value) -> CmlTask {
    parse_cml(&serde_json::to_string(value).expect("serialize fixture"))
        .expect("fixture must be strict-valid CML")
}

/// R1: liveness is recomputed at `observed_at` from
/// `last_heartbeat_at`/`ttl_seconds`: age<=ttl Online, <=2*ttl Stale, else
/// Offline; no heartbeat => Offline. Stored presence is `online` in every
/// case, so agreement with the stored field would fail the boundaries.
#[test]
fn r1_liveness_recomputed_at_observed_at() {
    let t0: u64 = 1_700_000_000;
    let doc = working_task_json();

    let mut offline_doc = doc.clone();
    offline_doc["runtime"]["last_heartbeat_at"] = Value::Null;
    offline_doc["runtime"]["presence"] = json!("offline");

    let cases = [
        // age 180 == ttl -> Online (boundary)
        (doc.clone(), t0 + 180, Presence::Online),
        // age 181 -> Stale
        (doc.clone(), t0 + 181, Presence::Stale),
        // age 360 == 2*ttl -> Stale (boundary)
        (doc.clone(), t0 + 360, Presence::Stale),
        // age 361 -> Offline
        (doc, t0 + 361, Presence::Offline),
        // no heartbeat at all -> Offline
        (offline_doc, t0 + 500, Presence::Offline),
    ];

    for (input, observed_at, expected) in cases {
        let task = parse(&input);
        let card = project_workstream_card(&task, observed_at);
        assert_eq!(
            card.liveness, expected,
            "liveness at observed_at={observed_at}"
        );
    }
}

/// R2: the stored `runtime.presence` (signed at `updated_at`) must not be
/// echoed. Stored Online (age 100 at updated_at), observed at age 300
/// -> must report Stale, not Online.
#[test]
fn r2_stored_presence_not_echoed() {
    let t0: u64 = 1_700_000_000;
    let task = parse(&working_task_json());
    assert_eq!(task.runtime.presence, Presence::Online, "fixture sanity");

    let card = project_workstream_card(&task, t0 + 300); // age 300: 180 < 300 <= 360
    assert_eq!(card.liveness, Presence::Stale);
    assert_ne!(card.liveness, task.runtime.presence);
}

/// R3: `live_claim` is true ONLY if liveness is Online AND
/// `lease.expires_at > observed_at`.
#[test]
fn r3_live_claim_requires_online_and_unexpired_lease() {
    let t0: u64 = 1_700_000_000;

    // Online + unexpired lease -> true
    let fresh = parse(&working_task_json()); // heartbeat t0, expires t0+10000
    let card = project_workstream_card(&fresh, t0 + 100); // age 100, lease live
    assert_eq!(card.liveness, Presence::Online, "sanity: online");
    assert!(card.live_claim, "online + unexpired lease => live_claim");

    // Online but lease already expired -> false
    let mut expired = working_task_json();
    expired["lease"]["expires_at"] = json!(t0 + 50);
    let card = project_workstream_card(&parse(&expired), t0 + 100);
    assert_eq!(card.liveness, Presence::Online, "sanity: still online");
    assert!(!card.live_claim, "expired lease => not live_claim");

    // Unexpired lease but stale heartbeat -> false
    let stale = parse(&working_task_json());
    let card = project_workstream_card(&stale, t0 + 300); // age 300 => Stale
    assert_eq!(card.liveness, Presence::Stale, "sanity: stale");
    assert!(!card.live_claim, "stale heartbeat => not live_claim");
}

/// R4: `head_short` is None when `git.head_sha` is None — no invented value.
#[test]
fn r4_head_short_none_when_head_sha_none() {
    let mut doc = working_task_json();
    // head_sha is already null in the base fixture
    let task = parse(&doc);
    assert!(task.git.head_sha.is_none(), "fixture sanity");
    let card = project_workstream_card(&task, 1_700_000_500);
    assert_eq!(card.head_short, None);
    // Same for a lease-less proposed task (no worker context at all).
    doc["status"] = json!("proposed");
    doc["lease"] = Value::Null;
    doc["roles"]["worker"] = Value::Null;
    doc["runtime"]["last_heartbeat_at"] = Value::Null;
    doc["runtime"]["presence"] = json!("offline");
    let task = parse(&doc);
    let card = project_workstream_card(&task, 1_700_000_500);
    assert_eq!(card.head_short, None);
}

/// R5: the serialized card contains no full 40-char SHA (only short forms).
#[test]
fn r5_no_full_sha_in_serialized_card() {
    let mut doc = working_task_json();
    doc["git"]["head_sha"] = json!(HEAD_SHA);
    let task = parse(&doc);
    let card = project_workstream_card(&task, 1_700_000_100);

    let serialized = serde_json::to_string(&card).expect("card must be serializable");
    assert!(
        !serialized.contains(BASE_SHA),
        "full base_sha leaked: {serialized}"
    );
    assert!(
        !serialized.contains(HEAD_SHA),
        "full head_sha leaked: {serialized}"
    );
    // No standalone 40-hex run anywhere (64-hex pubkeys are not SHAs).
    let max_run = longest_hex_run(&serialized);
    assert_ne!(
        max_run, 40,
        "40-char hex run found in card JSON: {serialized}"
    );
}

fn longest_hex_run(s: &str) -> usize {
    let mut best = 0;
    let mut cur = 0;
    for byte in s.bytes() {
        if byte.is_ascii_hexdigit() {
            cur += 1;
            best = best.max(cur);
        } else {
            cur = 0;
        }
    }
    best
}
