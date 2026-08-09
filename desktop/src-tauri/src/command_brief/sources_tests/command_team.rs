use super::*;

#[test]
fn fresh_collection_freezes_one_snapshot_and_all_local_source_kinds() {
    let backend = FakeBackend::fresh();
    let context = collector(backend, "Prepare today's command brief.")
        .freeze()
        .expect("fresh collection");

    assert_eq!(context.snapshot_id(), SNAPSHOT_A);
    assert_eq!(context.observed_at(), OBSERVED_AT);
    assert_eq!(context.rag_catalogue(), &["navy-publications".to_string()]);
    assert!(context.degraded_sections().is_empty());
    assert!(context
        .ledger()
        .iter()
        .all(|source| source.snapshot_id() == SNAPSHOT_A));
    let kinds = source_kinds(&context);
    for expected in [
        SourceKind::Rag,
        SourceKind::Memory,
        SourceKind::Calendar,
        SourceKind::Reminders,
        SourceKind::Notes,
        SourceKind::File,
    ] {
        assert!(kinds.contains(&expected), "missing {expected:?}");
    }
    assert!(context
        .validated_sources()
        .iter()
        .all(|source| source.snapshot_id() == SNAPSHOT_A));
}

#[test]
fn command_team_discussion_outcome_is_cited_as_memory_without_degrading_the_brief() {
    let backend = FakeBackend::with_state(|state| {
        state.command_team_discussions = CommandTeamDiscussionBatch::for_test(1, Vec::new());
    });

    let context = collector(backend, "Prepare today's command brief.")
        .freeze()
        .expect("discussion outcome remains an optional source");

    let discussion = context
        .ledger()
        .iter()
        .find(|source| source.collection() == "command_team_discussions")
        .expect("validated discussion source");
    assert_eq!(discussion.source_kind(), SourceKind::Memory);
    assert!(discussion.location().contains("builtin:command-operations"));
    assert!(context.degraded_sections().is_empty());
}

#[test]
fn command_team_discussion_failure_is_a_warning_without_section_degradation() {
    let backend = FakeBackend::with_state(|state| {
        state.command_team_discussions = CommandTeamDiscussionBatch::for_test(
            0,
            vec!["Command-team discussion memory was unavailable.".to_string()],
        );
    });

    let context = collector(backend, "Prepare today's command brief.")
        .freeze()
        .expect("discussion failure remains fail soft");

    assert!(context
        .limitations()
        .iter()
        .any(|item| item.contains("discussion memory was unavailable")));
    assert!(context.degraded_sections().is_empty());
}

#[test]
fn signed_battle_rhythm_and_plan_state_enter_the_brief_as_distinct_sources() {
    let backend = FakeBackend::with_state(|state| {
        state.planning_evidence = PlanningEvidenceBatch::for_test();
    });

    let context = collector(backend, "Prepare today's command brief.")
        .freeze()
        .expect("planning evidence remains an optional source");

    let battle_rhythm = context
        .ledger()
        .iter()
        .find(|source| source.collection() == "battle_rhythm")
        .expect("Battle Rhythm source");
    assert_eq!(battle_rhythm.source_kind(), SourceKind::BattleRhythm);
    assert!(battle_rhythm.quote().contains("Sail Manila"));

    let plan = context
        .ledger()
        .iter()
        .find(|source| source.collection() == "command_plans")
        .expect("Plans source");
    assert_eq!(plan.source_kind(), SourceKind::Plans);
    assert!(plan.quote().contains("Repair davit"));
    assert!(context.degraded_sections().is_empty());
}
