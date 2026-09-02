use buzz_core::cml::{parse_cml, CmlTask, Presence};
use buzz_core::cml_view::project_workstream_card;

const WORKER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const PLANNER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BASE_SHA: &str = "1111111111111111111111111111111111111111";
const HEAD_SHA: &str = "abcdef0123456789abcdef0123456789abcdef01";
const UPDATED_AT: u64 = 1_787_673_000;
const TTL: u64 = 180;

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

#[test]
fn fresh_heartbeat_with_valid_lease_is_a_live_claim() {
    let task = claimed_task(Some(60), UPDATED_AT + 600, Some(HEAD_SHA));
    let card = project_workstream_card(&task, UPDATED_AT);
    assert_eq!(card.liveness, Presence::Online);
    assert!(card.live_claim, "fresh heartbeat + valid lease is live");
}

#[test]
fn heartbeat_older_than_one_ttl_is_stale_and_not_a_live_claim() {
    // Heartbeat 240s old at observation: > TTL, <= 2*TTL.
    let task = claimed_task(Some(60), UPDATED_AT + 600, Some(HEAD_SHA));
    let card = project_workstream_card(&task, UPDATED_AT + 180);
    assert_eq!(card.liveness, Presence::Stale);
    assert!(!card.live_claim, "stale heartbeat is not a live claim");
}

#[test]
fn heartbeat_older_than_two_ttls_is_offline() {
    let task = claimed_task(Some(60), UPDATED_AT + 6000, Some(HEAD_SHA));
    let card = project_workstream_card(&task, UPDATED_AT + 400);
    assert_eq!(card.liveness, Presence::Offline);
    assert!(!card.live_claim);
}

#[test]
fn absent_heartbeat_is_offline() {
    let task = claimed_task(None, UPDATED_AT + 600, Some(HEAD_SHA));
    let card = project_workstream_card(&task, UPDATED_AT);
    assert_eq!(card.liveness, Presence::Offline);
    assert!(!card.live_claim);
}

#[test]
fn expired_lease_is_not_a_live_claim_even_with_a_fresh_heartbeat() {
    let task = claimed_task(Some(10), UPDATED_AT - 1, Some(HEAD_SHA));
    let card = project_workstream_card(&task, UPDATED_AT);
    assert_eq!(card.liveness, Presence::Online, "heartbeat is still fresh");
    assert!(!card.live_claim, "an expired lease is not a live claim");
}

/// The regression this projection exists to prevent: the stored `presence`
/// field is only correct at `updated_at`, so a card must never echo it.
#[test]
fn card_does_not_echo_the_stored_presence_field() {
    let task = claimed_task(Some(60), UPDATED_AT + 600, Some(HEAD_SHA));
    assert_eq!(
        task.runtime.presence,
        Presence::Online,
        "snapshot is signed Online"
    );
    let card = project_workstream_card(&task, UPDATED_AT + 200);
    assert_eq!(
        card.liveness,
        Presence::Stale,
        "observation-time liveness must override the signed snapshot value"
    );
}

#[test]
fn git_metadata_is_surfaced_and_missing_head_is_not_invented() {
    let with_head = claimed_task(Some(60), UPDATED_AT + 600, Some(HEAD_SHA));
    let card = project_workstream_card(&with_head, UPDATED_AT);
    assert_eq!(card.repo, "block/buzz");
    assert_eq!(card.branch, "feat/cml-view");
    assert_eq!(card.base_short, "1111111");
    assert_eq!(card.head_short.as_deref(), Some("abcdef0"));
    assert_eq!(card.worktree_alias, "buzz-s4");
    assert_eq!(card.host_id.as_deref(), Some("h_0123456789abcdef"));

    let without_head = claimed_task(Some(60), UPDATED_AT + 600, None);
    let card = project_workstream_card(&without_head, UPDATED_AT);
    assert_eq!(card.head_short, None, "missing head must not be invented");
}

#[test]
fn projection_never_leaks_absolute_paths_or_full_shas() {
    let task = claimed_task(Some(60), UPDATED_AT + 600, Some(HEAD_SHA));
    let card = project_workstream_card(&task, UPDATED_AT);
    let rendered = serde_json::to_string(&card).expect("card serializes");
    assert!(
        !rendered.contains("/Users/") && !rendered.contains("/private/"),
        "no absolute filesystem path may appear: {rendered}"
    );
    assert!(
        !rendered.contains(BASE_SHA),
        "full base SHA must be shortened: {rendered}"
    );
    assert!(
        !rendered.contains(HEAD_SHA),
        "full head SHA must be shortened: {rendered}"
    );
}
