use std::path::Path;

use buzz_core::agent_skill::{SkillTestV1, SkillVersionV1};
use nostr::Keys;
use tempfile::TempDir;

use crate::skill_learning::{
    evaluate::evaluate_candidate,
    registry::{PublicationKind, PublicationState},
    LearningAction, LearningOutcome, SkillLearningRuntime, TurnLearningEvidence,
};

fn evidence(id: &str, task: &str, outcome: LearningOutcome) -> TurnLearningEvidence {
    TurnLearningEvidence {
        experience_id: id.to_string(),
        occurred_at: "2026-08-17T00:00:00Z".to_string(),
        task_text: task.to_string(),
        outcome,
    }
}

fn open_runtime(path: &Path) -> SkillLearningRuntime {
    SkillLearningRuntime::open(
        path,
        Keys::generate(),
        Keys::generate().public_key(),
        "operations-adviser",
    )
    .expect("open learning runtime")
}

fn promote_once(runtime: &SkillLearningRuntime, prefix: &str, task: &str) -> (String, String) {
    assert_eq!(
        runtime
            .observe_turn(evidence(
                &format!("{prefix}-1"),
                task,
                LearningOutcome::Succeeded,
            ))
            .expect("first observation"),
        LearningAction::None
    );
    let action = runtime
        .observe_turn(evidence(
            &format!("{prefix}-2"),
            task,
            LearningOutcome::Succeeded,
        ))
        .expect("second observation");
    let LearningAction::Promoted {
        skill_id,
        version_id,
    } = action
    else {
        panic!("second matching success should queue promotion");
    };
    (skill_id, version_id)
}

fn publish_and_materialize(runtime: &SkillLearningRuntime, version_id: &str) {
    let version = runtime
        .registry()
        .ready_for_publish()
        .expect("version work");
    assert_eq!(version.len(), 1);
    assert_eq!(version[0].kind, PublicationKind::Version);
    runtime
        .registry()
        .mark_version_published(version_id)
        .expect("version acknowledgement");

    let pointer = runtime
        .registry()
        .ready_for_publish()
        .expect("pointer work");
    assert_eq!(pointer.len(), 1);
    assert_eq!(pointer[0].kind, PublicationKind::Pointer);
    runtime
        .registry()
        .mark_pointer_published(version_id)
        .expect("pointer acknowledgement");
    runtime
        .registry()
        .mark_materialized(version_id)
        .expect("materialization checkpoint");
}

#[test]
fn two_distinct_matching_successes_queue_one_candidate() {
    let temp = TempDir::new().expect("tempdir");
    let runtime = open_runtime(&temp.path().join("skills.sqlite"));
    let task = "Prepare the sailing readiness checklist for serial 4819";

    assert_eq!(
        runtime
            .observe_turn(evidence("exp-1", task, LearningOutcome::Succeeded))
            .expect("first observation"),
        LearningAction::None
    );
    let action = runtime
        .observe_turn(evidence(
            "exp-2",
            "Prepare the sailing readiness checklist for serial 9921",
            LearningOutcome::Succeeded,
        ))
        .expect("second observation");
    assert!(matches!(action, LearningAction::Promoted { .. }));

    let work = runtime
        .registry()
        .ready_for_publish()
        .expect("publication work");
    assert_eq!(work.len(), 1);
    assert_eq!(work[0].kind, PublicationKind::Version);

    assert_eq!(
        runtime
            .observe_turn(evidence("exp-2", task, LearningOutcome::Succeeded))
            .expect("duplicate delivery"),
        LearningAction::None
    );
    assert_eq!(
        runtime
            .registry()
            .ready_for_publish()
            .expect("publication work")
            .len(),
        1
    );
}

#[test]
fn pending_publication_blocks_parallel_candidate_for_same_skill() {
    let temp = TempDir::new().expect("tempdir");
    let runtime = open_runtime(&temp.path().join("skills.sqlite"));
    let task = "Prepare the sailing readiness checklist";
    let _ = promote_once(&runtime, "first", task);

    assert_eq!(
        runtime
            .observe_turn(evidence("later-1", task, LearningOutcome::Succeeded))
            .expect("third observation"),
        LearningAction::None
    );
    assert_eq!(
        runtime
            .observe_turn(evidence("later-2", task, LearningOutcome::Succeeded))
            .expect("fourth observation"),
        LearningAction::None
    );
    assert_eq!(
        runtime
            .registry()
            .ready_for_publish()
            .expect("one pending version")
            .len(),
        1
    );
}

#[test]
fn different_tasks_do_not_combine() {
    let temp = TempDir::new().expect("tempdir");
    let runtime = open_runtime(&temp.path().join("skills.sqlite"));

    for (id, task) in [
        ("exp-a", "Prepare the sailing readiness checklist"),
        ("exp-b", "Draft the monthly logistics report"),
    ] {
        assert_eq!(
            runtime
                .observe_turn(evidence(id, task, LearningOutcome::Succeeded))
                .expect("observation"),
            LearningAction::None
        );
    }
    assert!(runtime
        .registry()
        .ready_for_publish()
        .expect("publication work")
        .is_empty());
}

#[test]
fn candidate_preserves_parent_checks_and_rejects_removed_checks() {
    let temp = TempDir::new().expect("tempdir");
    let runtime = open_runtime(&temp.path().join("skills.sqlite"));
    let task = "Prepare the sailing readiness checklist";
    let (_, first_version) = promote_once(&runtime, "first", task);
    publish_and_materialize(&runtime, &first_version);

    let (_, second_version) = promote_once(&runtime, "second", task);
    let child = runtime
        .registry()
        .version(&second_version)
        .expect("load child")
        .expect("child exists");
    let parent = runtime
        .registry()
        .version(&first_version)
        .expect("load parent")
        .expect("parent exists");
    assert_eq!(
        child.inherited_tests,
        parent
            .inherited_tests
            .iter()
            .chain(parent.regression_tests.iter())
            .cloned()
            .collect::<Vec<_>>()
    );

    let mut stripped = child.clone();
    stripped.inherited_tests.clear();
    assert!(evaluate_candidate(&stripped, Some(&parent)).is_err());
}

#[test]
fn prohibited_candidate_text_never_changes_the_pointer() {
    let temp = TempDir::new().expect("tempdir");
    let runtime = open_runtime(&temp.path().join("skills.sqlite"));
    let unsafe_task = "Store an API key and add a new endpoint for automatic external action";

    assert_eq!(
        runtime
            .observe_turn(evidence(
                "unsafe-1",
                unsafe_task,
                LearningOutcome::Succeeded,
            ))
            .expect("first unsafe observation"),
        LearningAction::None
    );
    assert_eq!(
        runtime
            .observe_turn(evidence(
                "unsafe-2",
                unsafe_task,
                LearningOutcome::Succeeded,
            ))
            .expect("second unsafe observation"),
        LearningAction::None
    );
    assert!(runtime
        .registry()
        .ready_for_publish()
        .expect("publication work")
        .is_empty());
}

#[test]
fn two_matching_failures_queue_rollback_to_parent() {
    let temp = TempDir::new().expect("tempdir");
    let runtime = open_runtime(&temp.path().join("skills.sqlite"));
    let task = "Prepare the sailing readiness checklist";
    let (_, first_version) = promote_once(&runtime, "first", task);
    publish_and_materialize(&runtime, &first_version);
    let (skill_id, second_version) = promote_once(&runtime, "second", task);
    publish_and_materialize(&runtime, &second_version);

    assert_eq!(
        runtime
            .observe_turn(evidence("fail-1", task, LearningOutcome::Failed))
            .expect("first failure"),
        LearningAction::None
    );
    let rollback = runtime
        .observe_turn(evidence("fail-2", task, LearningOutcome::Failed))
        .expect("second failure");
    assert_eq!(
        rollback,
        LearningAction::RolledBack {
            skill_id: skill_id.clone(),
            version_id: first_version.clone(),
        }
    );
    assert_eq!(
        runtime
            .registry()
            .active_version(&skill_id)
            .expect("active pointer before ack"),
        Some(second_version)
    );
    let work = runtime
        .registry()
        .ready_for_publish()
        .expect("rollback pointer work");
    assert_eq!(work.len(), 1);
    assert_eq!(work[0].kind, PublicationKind::Pointer);
}

#[test]
fn unrelated_failure_does_not_affect_active_skill() {
    let temp = TempDir::new().expect("tempdir");
    let runtime = open_runtime(&temp.path().join("skills.sqlite"));
    let task = "Prepare the sailing readiness checklist";
    let (skill_id, version_id) = promote_once(&runtime, "first", task);
    publish_and_materialize(&runtime, &version_id);

    assert_eq!(
        runtime
            .observe_turn(evidence(
                "failure-other",
                "Draft the monthly logistics report",
                LearningOutcome::Failed,
            ))
            .expect("unrelated failure"),
        LearningAction::None
    );
    assert_eq!(
        runtime
            .registry()
            .active_version(&skill_id)
            .expect("active version"),
        Some(version_id)
    );
}

#[test]
fn outbox_resumes_each_checkpoint_without_duplicate_events() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("skills.sqlite");
    let runtime = open_runtime(&path);
    let (_, version_id) = promote_once(
        &runtime,
        "resume",
        "Prepare the sailing readiness checklist",
    );
    let version_event_id = runtime
        .registry()
        .ready_for_publish()
        .expect("version work")[0]
        .event
        .id
        .to_hex();
    drop(runtime);

    let reopened = open_runtime(&path);
    let pending = reopened
        .registry()
        .ready_for_publish()
        .expect("pending after restart");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].event.id.to_hex(), version_event_id);
    reopened
        .registry()
        .mark_version_published(&version_id)
        .expect("version acknowledgement");
    drop(reopened);

    let reopened = open_runtime(&path);
    let pointer = reopened
        .registry()
        .ready_for_publish()
        .expect("pointer after restart");
    assert_eq!(pointer.len(), 1);
    assert_eq!(pointer[0].kind, PublicationKind::Pointer);
    let pointer_event_id = pointer[0].event.id.to_hex();
    drop(reopened);

    let reopened = open_runtime(&path);
    assert_eq!(
        reopened.registry().ready_for_publish().expect("pointer")[0]
            .event
            .id
            .to_hex(),
        pointer_event_id
    );
    reopened
        .registry()
        .mark_pointer_published(&version_id)
        .expect("pointer acknowledgement");
    assert_eq!(
        reopened
            .registry()
            .publication_state(&version_id)
            .expect("state"),
        Some(PublicationState::PointerPublished)
    );
    drop(reopened);

    let reopened = open_runtime(&path);
    assert!(reopened
        .registry()
        .ready_for_publish()
        .expect("nothing publishable")
        .is_empty());
    reopened
        .registry()
        .mark_materialized(&version_id)
        .expect("materialized");
    assert_eq!(
        reopened
            .registry()
            .publication_state(&version_id)
            .expect("state"),
        Some(PublicationState::Materialized)
    );
}

#[test]
fn duplicate_experience_with_divergent_bytes_is_rejected() {
    let temp = TempDir::new().expect("tempdir");
    let runtime = open_runtime(&temp.path().join("skills.sqlite"));
    runtime
        .observe_turn(evidence(
            "duplicate",
            "Prepare the sailing readiness checklist",
            LearningOutcome::Succeeded,
        ))
        .expect("first observation");
    assert!(runtime
        .observe_turn(evidence(
            "duplicate",
            "Draft the monthly logistics report",
            LearningOutcome::Succeeded,
        ))
        .is_err());
}

#[test]
fn evaluator_requires_inherited_checks_byte_for_byte() {
    let inherited = SkillTestV1 {
        check_id: "required-heading".to_string(),
        kind: "contains".to_string(),
        expected: "# Procedure".to_string(),
    };
    let parent = SkillVersionV1 {
        skill_id: "learned-0123456789ab".to_string(),
        version_id: "version-parent".to_string(),
        parent_version_id: None,
        scope: buzz_core::agent_skill::SkillScope::SpecialistPrivate,
        specialist_id: Some("operations-adviser".to_string()),
        team_id: None,
        created_at: "2026-08-17T00:00:00Z".to_string(),
        source_experience_ids: vec!["source-a".to_string(), "source-b".to_string()],
        required_tools: vec![],
        inherited_tests: vec![],
        regression_tests: vec![inherited.clone()],
        skill_md: "---\nname: learned-0123456789ab\ndescription: test\n---\n# Procedure\nSafe."
            .to_string(),
        content_hash: String::new(),
    };
    let mut parent = parent;
    parent.content_hash = buzz_core::agent_skill::skill_body_hash(&parent.skill_md);
    let mut child = parent.clone();
    child.version_id = "version-child".to_string();
    child.parent_version_id = Some(parent.version_id.clone());
    child.source_experience_ids = vec!["source-c".to_string(), "source-d".to_string()];
    child.inherited_tests = vec![inherited];
    child.regression_tests.clear();
    assert!(evaluate_candidate(&child, Some(&parent)).is_ok());

    child.inherited_tests[0].expected = "# Different".to_string();
    assert!(evaluate_candidate(&child, Some(&parent)).is_err());

    let mut misplaced_name = parent.clone();
    misplaced_name.skill_md = misplaced_name.skill_md.replacen(
        "name: learned-0123456789ab",
        "title: learned-0123456789ab",
        1,
    ) + "\nname: learned-0123456789ab\n";
    misplaced_name.content_hash = buzz_core::agent_skill::skill_body_hash(&misplaced_name.skill_md);
    assert!(evaluate_candidate(&misplaced_name, None).is_err());
}
