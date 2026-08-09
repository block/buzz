use super::personas::{definition_for, specialist_definitions, PersonaDefinition};
use super::types::{AdviserId, BriefSection, SourceKind, ADVISORY_LIMITATION};

#[test]
fn pins_the_exact_eight_native_personas_and_specialist_order() {
    let roster = [
        AdviserId::ChiefOfStaff,
        AdviserId::Operations,
        AdviserId::Intelligence,
        AdviserId::Logistics,
        AdviserId::Navigation,
        AdviserId::DailyRoutine,
        AdviserId::Reporting,
        AdviserId::Plans,
    ];
    assert_eq!(
        roster.map(|adviser| definition_for(adviser).adviser),
        roster,
        "the Rust-owned roster must be closed and complete"
    );
    assert_eq!(
        specialist_definitions()
            .iter()
            .map(|definition| definition.adviser)
            .collect::<Vec<_>>(),
        vec![
            AdviserId::Operations,
            AdviserId::Intelligence,
            AdviserId::Logistics,
            AdviserId::Navigation,
            AdviserId::DailyRoutine,
            AdviserId::Reporting,
            AdviserId::Plans,
        ]
    );
}

#[test]
fn persona_tool_and_source_policy_is_owned_by_rust() {
    let operations = definition_for(AdviserId::Operations);
    for adviser in [
        AdviserId::Operations,
        AdviserId::Navigation,
        AdviserId::Reporting,
        AdviserId::Plans,
    ] {
        let definition = definition_for(adviser);
        assert_eq!(
            definition.permitted_source_kinds,
            &[
                SourceKind::Rag,
                SourceKind::Memory,
                SourceKind::BattleRhythm,
                SourceKind::Plans
            ]
        );
        assert_eq!(definition.permitted_tool_labels, &["memory", "rag"]);
    }

    for adviser in [AdviserId::Intelligence, AdviserId::Logistics] {
        let definition = definition_for(adviser);
        assert_eq!(
            definition.permitted_source_kinds,
            &[
                SourceKind::Rag,
                SourceKind::Memory,
                SourceKind::WorldMonitor,
                SourceKind::BattleRhythm,
                SourceKind::Plans
            ]
        );
    }
    assert_eq!(
        definition_for(AdviserId::Intelligence).permitted_tool_labels,
        &["memory", "rag", "world_monitor"]
    );

    let routine = definition_for(AdviserId::DailyRoutine);
    assert_eq!(
        routine.permitted_source_kinds,
        &[
            SourceKind::Rag,
            SourceKind::Memory,
            SourceKind::BattleRhythm,
            SourceKind::Plans,
            SourceKind::Calendar,
            SourceKind::Reminders,
            SourceKind::Notes,
            SourceKind::File,
        ]
    );
    assert_eq!(routine.permitted_tool_labels, &["apple", "memory", "rag"]);

    let chief = definition_for(AdviserId::ChiefOfStaff);
    assert!(chief.permitted_source_kinds.is_empty());
    assert!(chief.permitted_tool_labels.is_empty());
    assert_eq!(
        chief.permitted_sections,
        &[
            BriefSection::Today,
            BriefSection::Decisions,
            BriefSection::ConflictsAndGaps,
            BriefSection::Sources,
        ]
    );
    assert_ne!(operations.purpose, chief.purpose);
}

#[test]
fn fixed_prompts_are_structured_advisory_and_cannot_be_renderer_overridden() {
    let navigation = definition_for(AdviserId::Navigation);
    let prompt = navigation.system_prompt();
    for required in [
        "Return exactly one JSON object",
        "\"sourceIds\"",
        "limitations",
        "dissent",
        "pending",
        ADVISORY_LIMITATION,
    ] {
        assert!(prompt.contains(required), "missing {required:?}");
    }
    assert!(prompt.contains("does not generate executable navigation orders"));
    assert_eq!(prompt.matches(ADVISORY_LIMITATION).count(), 1);
    assert_eq!(
        prompt
            .matches(
                "Navigation content identifies considerations and source limitations; it does not generate executable navigation orders or make navigational decisions.",
            )
            .count(),
        1
    );

    assert_no_renderer_control_surface(navigation);
}

#[test]
fn every_model_prompt_spells_out_the_exact_rust_output_contract() {
    for adviser in [
        AdviserId::Operations,
        AdviserId::Intelligence,
        AdviserId::Logistics,
        AdviserId::Navigation,
        AdviserId::DailyRoutine,
        AdviserId::Reporting,
        AdviserId::Plans,
    ] {
        let prompt = definition_for(adviser).system_prompt();
        for field in [
            "\"classification\":\"OFFICIAL\"",
            "\"adviser\"",
            "\"section\"",
            "\"findings\"",
            "\"sourceIds\"",
            "\"confidence\"",
            "\"limitations\"",
            "\"dissent\"",
            "\"proposedActions\"",
            "\"approvalState\":\"pending\"",
        ] {
            assert!(
                prompt.contains(field),
                "{adviser:?} prompt omitted required contract field {field}"
            );
        }
        assert!(prompt.contains("signed Battle Rhythm and command plan"));
        assert!(prompt.contains("pending proposal"));
    }

    let chief = definition_for(AdviserId::ChiefOfStaff).system_prompt();
    for field in [
        "\"classification\":\"OFFICIAL\"",
        "\"adviser\":\"chief_of_staff\"",
        "\"findings\"",
        "\"sourceIds\"",
        "\"limitations\"",
        "\"dissent\"",
    ] {
        assert!(
            chief.contains(field),
            "Chief prompt omitted required contract field {field}"
        );
    }
}

fn assert_no_renderer_control_surface(definition: &PersonaDefinition) {
    let prompt = definition.system_prompt();
    assert_eq!(
        prompt,
        definition_for(AdviserId::Navigation).system_prompt()
    );
    assert!(definition.output_schema_instruction.contains("JSON"));
    assert!(definition.safety_boundary.contains("untrusted evidence"));
}
