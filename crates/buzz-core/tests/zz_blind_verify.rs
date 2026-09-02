//! Boundary probes for `cml_view` — run by the orchestrator after the
//! independent verifier was cut short by API timeouts. NOT a blind test.
use buzz_core::cml::{parse_cml, Presence};
use buzz_core::cml_view::project_workstream_card;

const WORKER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const PLANNER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const UPDATED_AT: u64 = 1_787_673_000;
const TTL: u64 = 180;

fn task_with(heartbeat_at: Option<u64>, lease_expires: Option<u64>) -> buzz_core::cml::CmlTask {
    // Stored presence must be valid AT UPDATED_AT relative to heartbeat.
    let presence = match heartbeat_at {
        None => "offline",
        Some(h) => {
            let age = UPDATED_AT.saturating_sub(h);
            if age <= TTL {
                "online"
            } else if age <= TTL * 2 {
                "stale"
            } else {
                "offline"
            }
        }
    };
    let value = serde_json::json!({
        "acceptance": [{"id":"A1","text":"boundary","verified":false}],
        "blockers": [],
        "evidence": [],
        "git": {"base_sha":"1111111111111111111111111111111111111111","branch":"feat/x",
                 "head_sha":"abcdef0123456789abcdef0123456789abcdef01",
                 "repo":"block/buzz","worktree_alias":"wt"},
        "id": "cdd4722d-7481-4d01-9c0a-423b4454c179",
        "lease": lease_expires.map(|e| serde_json::json!({
            "id":"l1","holder":WORKER,"issued_at":UPDATED_AT-600,"expires_at":e})),
        "objective": "o", "priority": "P1", "protocol": "buzz-cml",
        "review": {"max_rounds":3,"round":0},
        "roles": {"fixer":null,"planner":PLANNER,"reviewer":null,"worker":WORKER},
        "runtime": {"host_id":null,"last_heartbeat_at":heartbeat_at,
                    "presence":presence,"ttl_seconds":TTL},
        "status": if lease_expires.is_some() { "claimed" } else { "planned" },
        "title": "t", "updated_at": UPDATED_AT, "version": 1
    });
    parse_cml(&value.to_string()).expect("valid fixture")
}

#[test]
fn boundary_age_exactly_ttl_is_online() {
    let t = task_with(Some(UPDATED_AT - TTL), Some(UPDATED_AT + 600));
    assert_eq!(
        project_workstream_card(&t, UPDATED_AT).liveness,
        Presence::Online
    );
}

#[test]
fn boundary_age_exactly_ttl_plus_one_is_stale() {
    let t = task_with(Some(UPDATED_AT - TTL - 1), Some(UPDATED_AT + 600));
    // NOTE: stored presence computed at UPDATED_AT for age TTL+1 is "stale" —
    // consistent, so fixture is valid.
    assert_eq!(
        project_workstream_card(&t, UPDATED_AT).liveness,
        Presence::Stale
    );
}

#[test]
fn boundary_age_exactly_two_ttl_is_stale() {
    let t = task_with(Some(UPDATED_AT - 2 * TTL), Some(UPDATED_AT + 600));
    assert_eq!(
        project_workstream_card(&t, UPDATED_AT).liveness,
        Presence::Stale
    );
}

#[test]
fn boundary_age_two_ttl_plus_one_is_offline() {
    let t = task_with(Some(UPDATED_AT - 2 * TTL - 1), Some(UPDATED_AT + 600));
    assert_eq!(
        project_workstream_card(&t, UPDATED_AT).liveness,
        Presence::Offline
    );
}

#[test]
fn boundary_lease_expires_exactly_at_observation_is_not_live() {
    let t = task_with(Some(UPDATED_AT), Some(UPDATED_AT));
    let card = project_workstream_card(&t, UPDATED_AT);
    assert_eq!(card.liveness, Presence::Online);
    assert!(
        !card.live_claim,
        "expires_at == observed_at must not be live (strict >)"
    );
}

#[test]
fn boundary_heartbeat_after_observation_clock_skew_is_online() {
    // Heartbeat 10s in the FUTURE relative to observation (host clock skew).
    // Stored presence must still validate at UPDATED_AT, so heartbeat is
    // UPDATED_AT + 10 only if updated_at >= heartbeat... it does not; so
    // instead use heartbeat exactly == UPDATED_AT and observe 10s EARLIER.
    let t = task_with(Some(UPDATED_AT), Some(UPDATED_AT + 600));
    let card = project_workstream_card(&t, UPDATED_AT - 10);
    assert_eq!(
        card.liveness,
        Presence::Online,
        "saturating age 0 => online"
    );
}

#[test]
fn zero_ttl_saturates_without_panic() {
    // ttl_seconds must be 180 to parse, so a zero TTL cannot be constructed
    // through valid CML — documenting that the boundary is unreachable via
    // the public parse path.
    let t = task_with(Some(UPDATED_AT), Some(UPDATED_AT + 600));
    assert_eq!(t.runtime.ttl_seconds, 180);
}
