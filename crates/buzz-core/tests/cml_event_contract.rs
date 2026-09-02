use buzz_core::cml::{
    CmlStatus, CmlTask, GitState, Presence, Priority, ReviewState, Roles, RuntimeState,
};
use buzz_core::cml_event::{reduce_cml_events, validate_cml_event, CmlRole, CmlTransition};
use buzz_core::kind::{KIND_JOB_ACCEPTED, KIND_JOB_PROGRESS, KIND_JOB_REQUEST};
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use uuid::Uuid;

fn key_hex(keys: &Keys) -> String {
    keys.public_key().to_hex()
}

fn task(
    status: CmlStatus,
    planner: &Keys,
    worker: Option<&Keys>,
    reviewer: Option<&Keys>,
) -> CmlTask {
    let worker_hex = worker.map(key_hex);
    CmlTask {
        acceptance: vec![],
        blockers: vec![],
        evidence: vec![],
        git: GitState {
            base_sha: "1".repeat(40),
            branch: "feat/cml-events".into(),
            head_sha: None,
            repo: "block/buzz".into(),
            worktree_alias: "buzz-cml-events".into(),
        },
        id: Uuid::parse_str("cdd4722d-7481-4d01-9c0a-423b4454c179").unwrap(),
        lease: if matches!(
            status,
            CmlStatus::Claimed | CmlStatus::Working | CmlStatus::Review
        ) {
            worker_hex.as_ref().map(|holder| buzz_core::cml::Lease {
                id: "lease-1".into(),
                holder: holder.clone(),
                issued_at: 1_787_673_000,
                expires_at: 1_787_674_000,
            })
        } else {
            None
        },
        objective: "Signed event reduction".into(),
        priority: Priority::P1,
        protocol: "buzz-cml".into(),
        review: ReviewState {
            max_rounds: 3,
            round: 0,
        },
        roles: Roles {
            fixer: None,
            planner: key_hex(planner),
            reviewer: reviewer.map(key_hex),
            worker: worker_hex,
        },
        runtime: RuntimeState {
            host_id: None,
            last_heartbeat_at: None,
            presence: Presence::Offline,
            ttl_seconds: 180,
        },
        status,
        title: "CML events".into(),
        updated_at: 1_787_673_000,
        version: 1,
        extensions: Default::default(),
    }
}

fn signed_event(
    keys: &Keys,
    kind: u32,
    snapshot: &CmlTask,
    transition: CmlTransition,
    role: CmlRole,
    previous: Option<&Event>,
) -> Event {
    let channel = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let mut tags = vec![
        Tag::parse(["h", channel]).unwrap(),
        Tag::parse(["d", &snapshot.id.to_string()]).unwrap(),
        Tag::parse(["protocol", "buzz-cml", "1"]).unwrap(),
        Tag::parse(["transition", transition.as_str()]).unwrap(),
        Tag::parse(["status", status_name(snapshot.status)]).unwrap(),
        Tag::parse(["role", role.as_str()]).unwrap(),
    ];
    if let Some(prev) = previous {
        tags.push(Tag::parse(["e", &prev.id.to_hex(), "prev"]).unwrap());
    }
    EventBuilder::new(
        Kind::Custom(kind as u16),
        snapshot.to_canonical_json().unwrap(),
    )
    .tags(tags)
    .custom_created_at(Timestamp::from(snapshot.updated_at))
    .sign_with_keys(keys)
    .unwrap()
}

fn signed_resolution(
    planner: &Keys,
    snapshot: &CmlTask,
    fork_a: &Event,
    fork_b: &Event,
    selected: &Event,
) -> Event {
    let channel = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let tags = vec![
        Tag::parse(["h", channel]).unwrap(),
        Tag::parse(["d", &snapshot.id.to_string()]).unwrap(),
        Tag::parse(["protocol", "buzz-cml", "1"]).unwrap(),
        Tag::parse(["transition", CmlTransition::OwnerResolve.as_str()]).unwrap(),
        Tag::parse(["status", status_name(snapshot.status)]).unwrap(),
        Tag::parse(["role", CmlRole::Planner.as_str()]).unwrap(),
        Tag::parse(["e", &fork_a.id.to_hex(), "fork_a"]).unwrap(),
        Tag::parse(["e", &fork_b.id.to_hex(), "fork_b"]).unwrap(),
        Tag::parse(["e", &selected.id.to_hex(), "selected"]).unwrap(),
    ];
    EventBuilder::new(
        Kind::Custom(KIND_JOB_PROGRESS as u16),
        snapshot.to_canonical_json().unwrap(),
    )
    .tags(tags)
    .custom_created_at(Timestamp::from(snapshot.updated_at))
    .sign_with_keys(planner)
    .unwrap()
}

fn status_name(status: CmlStatus) -> &'static str {
    match status {
        CmlStatus::Proposed => "proposed",
        CmlStatus::Planned => "planned",
        CmlStatus::Claimed => "claimed",
        CmlStatus::Working => "working",
        CmlStatus::Blocked => "blocked",
        CmlStatus::Review => "review",
        CmlStatus::Fixing => "fixing",
        CmlStatus::Verified => "verified",
        CmlStatus::Integrated => "integrated",
        CmlStatus::Shipped => "shipped",
        CmlStatus::Cancelled => "cancelled",
        CmlStatus::Conflicted => "conflicted",
    }
}

#[test]
fn signed_plan_and_claim_reduce_deterministically_out_of_order() {
    let planner = Keys::generate();
    let worker = Keys::generate();
    let plan = signed_event(
        &planner,
        KIND_JOB_REQUEST,
        &task(CmlStatus::Planned, &planner, Some(&worker), None),
        CmlTransition::Plan,
        CmlRole::Planner,
        None,
    );
    let claim = signed_event(
        &worker,
        KIND_JOB_ACCEPTED,
        &task(CmlStatus::Claimed, &planner, Some(&worker), None),
        CmlTransition::Claim,
        CmlRole::Worker,
        Some(&plan),
    );

    let reduced = reduce_cml_events(&[claim.clone(), plan.clone()]).expect("reduce");
    assert_eq!(reduced.task.status, CmlStatus::Claimed);
    assert_eq!(reduced.head, claim.id);
    assert!(!reduced.conflicted);
    validate_cml_event(&plan).expect("valid plan");
    validate_cml_event(&claim).expect("valid claim");
}

#[test]
fn forged_actor_and_tag_content_disagreement_fail_closed() {
    let planner = Keys::generate();
    let worker = Keys::generate();
    let attacker = Keys::generate();
    let snapshot = task(CmlStatus::Claimed, &planner, Some(&worker), None);
    let forged = signed_event(
        &attacker,
        KIND_JOB_ACCEPTED,
        &snapshot,
        CmlTransition::Claim,
        CmlRole::Worker,
        None,
    );
    assert!(validate_cml_event(&forged).is_err());

    let wrong_kind = signed_event(
        &planner,
        KIND_JOB_PROGRESS,
        &task(CmlStatus::Planned, &planner, Some(&worker), None),
        CmlTransition::Plan,
        CmlRole::Planner,
        None,
    );
    assert!(validate_cml_event(&wrong_kind).is_err());
}

#[test]
fn worker_cannot_rewrite_planner_contract_or_advance_review_round() {
    let planner = Keys::generate();
    let worker = Keys::generate();
    let plan = signed_event(
        &planner,
        KIND_JOB_REQUEST,
        &task(CmlStatus::Planned, &planner, Some(&worker), None),
        CmlTransition::Plan,
        CmlRole::Planner,
        None,
    );
    let mut rewritten = task(CmlStatus::Claimed, &planner, Some(&worker), None);
    rewritten.objective = "Worker-controlled objective".into();
    let claim = signed_event(
        &worker,
        KIND_JOB_ACCEPTED,
        &rewritten,
        CmlTransition::Claim,
        CmlRole::Worker,
        Some(&plan),
    );
    assert!(reduce_cml_events(&[plan.clone(), claim]).is_err());

    let mut jumped_round = task(CmlStatus::Claimed, &planner, Some(&worker), None);
    jumped_round.review.round = 1;
    let claim = signed_event(
        &worker,
        KIND_JOB_ACCEPTED,
        &jumped_round,
        CmlTransition::Claim,
        CmlRole::Worker,
        Some(&plan),
    );
    assert!(reduce_cml_events(&[plan, claim]).is_err());
}

#[test]
fn sibling_successors_are_exposed_as_conflict_not_last_writer_wins() {
    let planner = Keys::generate();
    let worker = Keys::generate();
    let plan = signed_event(
        &planner,
        KIND_JOB_REQUEST,
        &task(CmlStatus::Planned, &planner, Some(&worker), None),
        CmlTransition::Plan,
        CmlRole::Planner,
        None,
    );
    let claim_a = signed_event(
        &worker,
        KIND_JOB_ACCEPTED,
        &task(CmlStatus::Claimed, &planner, Some(&worker), None),
        CmlTransition::Claim,
        CmlRole::Worker,
        Some(&plan),
    );
    let mut fork_b_snapshot = task(CmlStatus::Claimed, &planner, Some(&worker), None);
    fork_b_snapshot.updated_at += 1;
    let claim_b = signed_event(
        &worker,
        KIND_JOB_ACCEPTED,
        &fork_b_snapshot,
        CmlTransition::Claim,
        CmlRole::Worker,
        Some(&plan),
    );

    let reduced = reduce_cml_events(&[plan, claim_b, claim_a]).expect("reduce conflict");
    assert!(reduced.conflicted);
    assert_eq!(reduced.task.status, CmlStatus::Conflicted);
}

#[test]
fn owner_resolution_selects_one_fork_without_rewriting_snapshot() {
    let planner = Keys::generate();
    let worker = Keys::generate();
    let plan = signed_event(
        &planner,
        KIND_JOB_REQUEST,
        &task(CmlStatus::Planned, &planner, Some(&worker), None),
        CmlTransition::Plan,
        CmlRole::Planner,
        None,
    );
    let selected_snapshot = task(CmlStatus::Claimed, &planner, Some(&worker), None);
    let selected = signed_event(
        &worker,
        KIND_JOB_ACCEPTED,
        &selected_snapshot,
        CmlTransition::Claim,
        CmlRole::Worker,
        Some(&plan),
    );
    let mut other_snapshot = selected_snapshot.clone();
    other_snapshot.updated_at += 1;
    let other = signed_event(
        &worker,
        KIND_JOB_ACCEPTED,
        &other_snapshot,
        CmlTransition::Claim,
        CmlRole::Worker,
        Some(&plan),
    );
    let mut resolution_snapshot = selected_snapshot;
    resolution_snapshot.updated_at += 2;
    let resolution =
        signed_resolution(&planner, &resolution_snapshot, &selected, &other, &selected);

    let reduced = reduce_cml_events(&[other, resolution.clone(), plan, selected]).expect("resolve");
    assert!(!reduced.conflicted);
    assert_eq!(reduced.task.status, CmlStatus::Claimed);
    assert_eq!(reduced.head, resolution.id);
}
