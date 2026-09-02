use buzz_core::cml::{
    CmlStatus, CmlTask, GitState, Presence, Priority, ReviewState, Roles, RuntimeState,
};
use buzz_core::cml_event::{CmlRole, CmlTransition};
use buzz_sdk::build_cml_transition;
use nostr::{Keys, Timestamp};
use uuid::Uuid;

fn planned_task(planner: &Keys) -> CmlTask {
    CmlTask {
        acceptance: vec![],
        blockers: vec![],
        evidence: vec![],
        git: GitState {
            base_sha: "1".repeat(40),
            branch: "feat/sdk-cml".into(),
            head_sha: None,
            repo: "block/buzz".into(),
            worktree_alias: "sdk-cml".into(),
        },
        id: Uuid::parse_str("cdd4722d-7481-4d01-9c0a-423b4454c179").unwrap(),
        lease: None,
        objective: "Typed signed transition".into(),
        priority: Priority::P1,
        protocol: "buzz-cml".into(),
        review: ReviewState {
            max_rounds: 3,
            round: 0,
        },
        roles: Roles {
            fixer: None,
            planner: planner.public_key().to_hex(),
            reviewer: None,
            worker: None,
        },
        runtime: RuntimeState {
            host_id: None,
            last_heartbeat_at: None,
            presence: Presence::Offline,
            ttl_seconds: 180,
        },
        status: CmlStatus::Planned,
        title: "SDK CML".into(),
        updated_at: 1_787_673_000,
        version: 1,
        extensions: Default::default(),
    }
}

#[test]
fn builder_emits_canonical_plan_tags_and_content() {
    let planner = Keys::generate();
    let task = planned_task(&planner);
    let event = build_cml_transition(
        Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap(),
        &task,
        CmlTransition::Plan,
        CmlRole::Planner,
        None,
    )
    .expect("build")
    .custom_created_at(Timestamp::from(task.updated_at))
    .sign_with_keys(&planner)
    .unwrap();

    assert_eq!(event.kind.as_u16(), 43001);
    assert_eq!(event.content, task.to_canonical_json().unwrap());
    assert!(event
        .tags
        .iter()
        .any(|tag| tag.as_slice() == ["transition", "planner.plan"]));
    assert!(event
        .tags
        .iter()
        .any(|tag| tag.as_slice() == ["protocol", "buzz-cml", "1"]));
}

#[test]
fn builder_rejects_wrong_role_and_missing_predecessor() {
    let planner = Keys::generate();
    let task = planned_task(&planner);
    let channel = Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap();
    assert!(
        build_cml_transition(channel, &task, CmlTransition::Plan, CmlRole::Worker, None).is_err()
    );
    assert!(
        build_cml_transition(channel, &task, CmlTransition::Claim, CmlRole::Worker, None).is_err()
    );
}
