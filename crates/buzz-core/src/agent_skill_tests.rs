use nostr::Keys;
use sha2::{Digest, Sha256};

use crate::agent_skill::{
    build_skill_pointer_event, build_skill_version_event, validate_and_decrypt_skill_pointer,
    validate_and_decrypt_skill_version, SkillPointerReason, SkillPointerV1, SkillScope,
    SkillTestV1, SkillVersionV1, MAX_SKILL_BODY_BYTES,
};

fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn version() -> SkillVersionV1 {
    let skill_md = "---\nname: learned-0123456789ab\ndescription: Reusable procedure.\n---\n\n# Procedure\n1. Check the current task.\n".to_string();
    SkillVersionV1 {
        skill_id: "learned-0123456789ab".into(),
        version_id: "version-0001".into(),
        parent_version_id: None,
        scope: SkillScope::SpecialistPrivate,
        specialist_id: Some("navigation".into()),
        team_id: None,
        created_at: "2026-08-17T01:02:03Z".into(),
        source_experience_ids: vec!["experience-1".into(), "experience-2".into()],
        required_tools: vec!["rag.search".into()],
        inherited_tests: vec![SkillTestV1 {
            check_id: "source-boundary".into(),
            kind: "contains".into(),
            expected: "current task".into(),
        }],
        regression_tests: vec![],
        content_hash: hash(&skill_md),
        skill_md,
    }
}

fn pointer() -> SkillPointerV1 {
    SkillPointerV1 {
        skill_id: "learned-0123456789ab".into(),
        active_version_id: "version-0001".into(),
        previous_version_id: None,
        scope: SkillScope::SpecialistPrivate,
        specialist_id: Some("navigation".into()),
        team_id: None,
        changed_at: "2026-08-17T01:03:04Z".into(),
        reason: SkillPointerReason::Promotion,
        evaluation_ids: vec!["evaluation-1".into()],
    }
}

#[test]
fn skill_version_round_trips_through_owner_encryption() {
    let agent = Keys::generate();
    let owner = Keys::generate();
    let expected = version();

    let event = build_skill_version_event(&agent, &owner.public_key(), &expected, 1_787_000_000)
        .expect("build version");
    let decoded = validate_and_decrypt_skill_version(
        &event,
        &agent.public_key(),
        &owner.public_key(),
        agent.secret_key(),
        &owner.public_key(),
    )
    .expect("decrypt version");

    assert_eq!(decoded, expected);
    assert_eq!(event.kind.as_u16(), 30180);
    assert_eq!(
        event
            .tags
            .iter()
            .filter(|tag| tag.kind().to_string() == "d")
            .count(),
        1
    );
    assert_eq!(
        event
            .tags
            .iter()
            .filter(|tag| tag.kind().to_string() == "p")
            .count(),
        1
    );
    assert!(!event.content.contains("current task"));
}

#[test]
fn skill_pointer_round_trips_and_uses_a_stable_address() {
    let agent = Keys::generate();
    let owner = Keys::generate();
    let first = pointer();
    let mut rollback = first.clone();
    rollback.active_version_id = "version-0000".into();
    rollback.previous_version_id = Some("version-0001".into());
    rollback.reason = SkillPointerReason::Rollback;

    let first_event = build_skill_pointer_event(&agent, &owner.public_key(), &first, 1_787_000_000)
        .expect("build first pointer");
    let rollback_event =
        build_skill_pointer_event(&agent, &owner.public_key(), &rollback, 1_787_000_001)
            .expect("build rollback pointer");
    let decoded = validate_and_decrypt_skill_pointer(
        &rollback_event,
        &agent.public_key(),
        &owner.public_key(),
        owner.secret_key(),
        &agent.public_key(),
    )
    .expect("decrypt pointer as owner");

    let d = |event: &nostr::Event| {
        event
            .tags
            .iter()
            .find(|tag| tag.kind().to_string() == "d")
            .and_then(|tag| tag.content())
            .expect("d tag")
            .to_string()
    };
    assert_eq!(decoded, rollback);
    assert_eq!(d(&first_event), d(&rollback_event));
}

#[test]
fn distinct_skill_versions_use_distinct_addresses() {
    let agent = Keys::generate();
    let owner = Keys::generate();
    let first = version();
    let mut second = first.clone();
    second.version_id = "version-0002".into();
    second.parent_version_id = Some(first.version_id.clone());

    let first_event =
        build_skill_version_event(&agent, &owner.public_key(), &first, 1).expect("first version");
    let second_event =
        build_skill_version_event(&agent, &owner.public_key(), &second, 2).expect("second version");
    let d = |event: &nostr::Event| {
        event
            .tags
            .iter()
            .find(|tag| tag.kind().to_string() == "d")
            .and_then(|tag| tag.content())
            .expect("d tag")
            .to_string()
    };
    assert_ne!(d(&first_event), d(&second_event));
}

#[test]
fn skill_contract_rejects_wrong_owner_and_tampered_ciphertext() {
    let agent = Keys::generate();
    let owner = Keys::generate();
    let stranger = Keys::generate();
    let event =
        build_skill_version_event(&agent, &owner.public_key(), &version(), 1).expect("version");

    assert!(validate_and_decrypt_skill_version(
        &event,
        &agent.public_key(),
        &stranger.public_key(),
        agent.secret_key(),
        &owner.public_key(),
    )
    .is_err());

    let mut tampered = event;
    tampered.content.push('x');
    assert!(validate_and_decrypt_skill_version(
        &tampered,
        &agent.public_key(),
        &owner.public_key(),
        agent.secret_key(),
        &owner.public_key(),
    )
    .is_err());
}

#[test]
fn skill_version_rejects_hash_lineage_size_and_duplicate_test_errors() {
    let mut invalid_hash = version();
    invalid_hash.content_hash = "0".repeat(64);
    assert!(invalid_hash.validate().is_err());

    let mut self_parent = version();
    self_parent.parent_version_id = Some(self_parent.version_id.clone());
    assert!(self_parent.validate().is_err());

    let mut oversized = version();
    oversized.skill_md = "x".repeat(MAX_SKILL_BODY_BYTES + 1);
    oversized.content_hash = hash(&oversized.skill_md);
    assert!(oversized.validate().is_err());

    let mut duplicate = version();
    duplicate
        .inherited_tests
        .push(duplicate.inherited_tests[0].clone());
    assert!(duplicate.validate().is_err());

    let mut aggregate_oversize = version();
    aggregate_oversize.inherited_tests = (0..20)
        .map(|index| SkillTestV1 {
            check_id: format!("check-{index}"),
            kind: "contains".into(),
            expected: "x".repeat(4_000),
        })
        .collect();
    assert!(aggregate_oversize.validate().is_err());
}

#[test]
fn skill_version_rejects_forbidden_tools_and_invalid_scope_coordinates() {
    let mut forbidden = version();
    forbidden.required_tools = vec!["shell.exec".into()];
    assert!(forbidden.validate().is_err());

    let mut missing_specialist = version();
    missing_specialist.specialist_id = None;
    assert!(missing_specialist.validate().is_err());

    let mut shared = version();
    shared.scope = SkillScope::CommandTeamShared;
    shared.specialist_id = None;
    shared.team_id = Some("command-team".into());
    assert!(shared.validate().is_ok());
}

#[test]
fn skill_pointer_rejects_self_transition_and_missing_scope_coordinate() {
    let mut self_transition = pointer();
    self_transition.previous_version_id = Some(self_transition.active_version_id.clone());
    assert!(self_transition.validate().is_err());

    let mut missing_specialist = pointer();
    missing_specialist.specialist_id = None;
    assert!(missing_specialist.validate().is_err());
}
