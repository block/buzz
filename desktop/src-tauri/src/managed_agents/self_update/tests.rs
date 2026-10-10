//! Pins every way a self-update can be refused or abused (#6287). Each test
//! names the failure it guards; removing the matching guard in
//! `evaluate_self_update` fails the test.

use super::*;
use crate::managed_agents::{AgentDefinition, ManagedAgentRecord};
use chrono::TimeZone;

const SIGNER: &str = "aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11";
const OTHER: &str = "bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22";

/// When the sample drafts were sent: after every fixture definition's last
/// write (2026-02-02), so the predates-definition rule does not fire by default.
fn issued() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 8, 12, 0, 0).unwrap()
}

fn definition(id: &str, display_name: &str) -> AgentDefinition {
    AgentDefinition {
        id: id.into(),
        display_name: display_name.into(),
        avatar_url: Some("https://example.com/a.png".into()),
        description: Some("A helper.".into()),
        system_prompt: "Be helpful.".into(),
        acp_command: Some("buzz-acp".into()),
        runtime: Some("goose".into()),
        model: Some("claude-sonnet-4".into()),
        provider: Some("anthropic".into()),
        name_pool: vec!["Birch".into()],
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        team_catalog_source: None,
        env_vars: Default::default(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        session_policy: Default::default(),
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-02-02T00:00:00Z".into(),
    }
}

fn record(
    pubkey: &str,
    persona_id: Option<&str>,
    policy: &[SelfUpdateField],
) -> ManagedAgentRecord {
    let mut record: ManagedAgentRecord = serde_json::from_str(&format!(
        r#"{{
            "pubkey": "{pubkey}",
            "name": "Scout",
            "persona_id": {persona},
            "relay_url": "wss://localhost:3000",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }}"#,
        persona = persona_id
            .map(|id| format!("\"{id}\""))
            .unwrap_or_else(|| "null".into()),
    ))
    .expect("sample record");
    record.self_update_fields = policy.to_vec();
    record
}

fn prompt_draft() -> SelfUpdateDraft {
    SelfUpdateDraft {
        channel_id: "7c07e659-3610-42f4-9a5e-1e9973c09da9".into(),
        agent_name: "Scout".into(),
        system_prompt: Some("  Be terse.  ".into()),
        ..Default::default()
    }
}

fn all_fields() -> Vec<SelfUpdateField> {
    vec![
        SelfUpdateField::SystemPrompt,
        SelfUpdateField::Model,
        SelfUpdateField::DisplayName,
    ]
}

// ── Policy persistence ───────────────────────────────────────────────────────

#[test]
fn old_store_without_the_field_loads_with_an_empty_policy() {
    let record = record(SIGNER, Some("scout"), &[]);
    assert!(record.self_update_fields.is_empty());
    let json = serde_json::to_string(&record).unwrap();
    assert!(
        !json.contains("self_update_fields"),
        "an empty policy must not change the stored bytes"
    );
}

#[test]
fn policy_round_trips_in_canonical_order_and_unknown_names_are_dropped() {
    let mut record = record(SIGNER, Some("scout"), &[]);
    record.self_update_fields = vec![SelfUpdateField::DisplayName, SelfUpdateField::SystemPrompt];
    let json = serde_json::to_string(&record).unwrap();
    assert!(json.contains(r#""self_update_fields":["display_name","system_prompt"]"#));

    let injected = json.replace(
        r#"["display_name","system_prompt"]"#,
        r#"["system_prompt","env_vars","system_prompt","runtime"]"#,
    );
    let parsed: ManagedAgentRecord = serde_json::from_str(&injected)
        .expect("a newer build's field names must not fail the whole store");
    assert_eq!(
        parsed.self_update_fields,
        vec![SelfUpdateField::SystemPrompt]
    );
}

#[test]
fn normalize_sorts_and_dedupes() {
    assert_eq!(
        normalize_self_update_fields(vec![
            SelfUpdateField::DisplayName,
            SelfUpdateField::SystemPrompt,
            SelfUpdateField::DisplayName,
        ]),
        vec![SelfUpdateField::SystemPrompt, SelfUpdateField::DisplayName]
    );
}

// ── Projection exclusion ─────────────────────────────────────────────────────

#[test]
fn policy_never_reaches_the_kind_30177_projection() {
    let record = record(SIGNER, Some("scout"), &all_fields());
    let json = serde_json::to_string(&crate::managed_agents::agent_events::agent_event_content(
        &record,
    ))
    .unwrap();
    assert!(
        !json.contains("self_update"),
        "policy leaked into kind:30177: {json}"
    );
}

#[test]
fn policy_never_reaches_an_agent_snapshot() {
    let mut record = record(SIGNER, Some("scout"), &all_fields());
    record.slug = Some("scout".into());
    record.system_prompt = Some("Be helpful.".into());
    let snapshot = crate::managed_agents::agent_snapshot::build_snapshot(
        &record,
        crate::managed_agents::agent_snapshot::MemoryLevel::None,
        vec![],
        None,
    );
    let json = serde_json::to_string(&snapshot).unwrap();
    assert!(
        !json.contains("self_update"),
        "policy leaked into agent snapshot: {json}"
    );
}

// ── Freshness ────────────────────────────────────────────────────────────────

#[test]
fn stale_undated_or_far_future_drafts_are_not_fresh() {
    let now = Utc.with_ymd_and_hms(2026, 10, 8, 12, 0, 0).unwrap();
    let fresh = |raw: Option<&str>| fresh_draft_issued_at(raw, now).is_some();
    assert!(fresh(Some("2026-10-08T11:55:00Z")));
    assert!(fresh(Some("2026-10-08T12:01:30+00:00")));
    assert!(!fresh(Some("2026-10-08T11:49:59Z")), "older than 10 min");
    assert!(!fresh(Some("2026-10-08T12:03:00Z")), "beyond skew");
    assert!(!fresh(Some("not a date")));
    assert!(!fresh(None));
}

// ── Authorization ────────────────────────────────────────────────────────────

#[test]
fn own_prompt_edit_within_policy_is_planned() {
    let records = [record(
        SIGNER,
        Some("scout"),
        &[SelfUpdateField::SystemPrompt],
    )];
    let definitions = [definition("scout", "Scout")];
    let plan =
        evaluate_self_update(SIGNER, &records, &definitions, &prompt_draft(), issued()).unwrap();
    assert_eq!(
        plan,
        SelfUpdatePlan {
            persona_id: "scout".into(),
            expected_updated_at: "2026-02-02T00:00:00Z".into(),
            display_name: None,
            system_prompt: Some("Be terse.".into()),
            model: None,
        }
    );
    assert_eq!(plan.fields(), vec![SelfUpdateField::SystemPrompt]);
}

#[test]
fn signer_pubkey_is_matched_case_insensitively() {
    let records = [record(
        SIGNER,
        Some("scout"),
        &[SelfUpdateField::SystemPrompt],
    )];
    let definitions = [definition("scout", "Scout")];
    assert!(evaluate_self_update(
        &SIGNER.to_uppercase(),
        &records,
        &definitions,
        &prompt_draft(),
        issued()
    )
    .is_ok());
}

#[test]
fn default_empty_policy_keeps_todays_review_path() {
    let records = [record(SIGNER, Some("scout"), &[])];
    let definitions = [definition("scout", "Scout")];
    assert_eq!(
        evaluate_self_update(SIGNER, &records, &definitions, &prompt_draft(), issued()),
        Err(SelfUpdateRejection::PolicyEmpty)
    );
}

#[test]
fn unknown_signer_is_rejected() {
    let records = [record(SIGNER, Some("scout"), &all_fields())];
    let definitions = [definition("scout", "Scout")];
    assert_eq!(
        evaluate_self_update(OTHER, &records, &definitions, &prompt_draft(), issued()),
        Err(SelfUpdateRejection::UnknownSigner)
    );
}

#[test]
fn a_different_agent_cannot_use_its_own_policy_on_someone_elses_definition() {
    // OTHER has a wide-open policy but drafts a change to Scout's definition.
    let records = [
        record(SIGNER, Some("scout"), &all_fields()),
        record(OTHER, Some("pip"), &all_fields()),
    ];
    let definitions = [definition("scout", "Scout"), definition("pip", "Pip")];
    assert_eq!(
        evaluate_self_update(OTHER, &records, &definitions, &prompt_draft(), issued()),
        Err(SelfUpdateRejection::TargetMismatch)
    );
}

#[test]
fn every_non_allowlisted_field_falls_back_to_review() {
    let records = [record(SIGNER, Some("scout"), &all_fields())];
    let definitions = [definition("scout", "Scout")];
    for (field, draft) in [
        (
            "runtime",
            SelfUpdateDraft {
                runtime: Some("claude".into()),
                ..prompt_draft()
            },
        ),
        (
            "provider",
            SelfUpdateDraft {
                provider: Some("openai".into()),
                ..prompt_draft()
            },
        ),
        (
            "respond_to",
            SelfUpdateDraft {
                respond_to: Some("anyone".into()),
                ..prompt_draft()
            },
        ),
    ] {
        assert_eq!(
            evaluate_self_update(SIGNER, &records, &definitions, &draft, issued()),
            Err(SelfUpdateRejection::FieldNotAllowed(field)),
            "{field} must never be self-updatable even with the widest policy"
        );
    }
}

#[test]
fn a_field_outside_the_agents_policy_rejects_the_whole_draft() {
    // Policy allows the prompt only; the draft also changes the model, so
    // nothing is applied (no silent partial apply).
    let records = [record(
        SIGNER,
        Some("scout"),
        &[SelfUpdateField::SystemPrompt],
    )];
    let definitions = [definition("scout", "Scout")];
    let draft = SelfUpdateDraft {
        model: Some("gpt-5".into()),
        ..prompt_draft()
    };
    assert_eq!(
        evaluate_self_update(SIGNER, &records, &definitions, &draft, issued()),
        Err(SelfUpdateRejection::FieldNotAllowed("model"))
    );
}

#[test]
fn blank_values_count_as_absent() {
    let records = [record(
        SIGNER,
        Some("scout"),
        &[SelfUpdateField::SystemPrompt],
    )];
    let definitions = [definition("scout", "Scout")];
    let draft = SelfUpdateDraft {
        model: Some("   ".into()),
        ..prompt_draft()
    };
    assert!(evaluate_self_update(SIGNER, &records, &definitions, &draft, issued()).is_ok());
    let empty = SelfUpdateDraft {
        system_prompt: Some("".into()),
        ..prompt_draft()
    };
    assert_eq!(
        evaluate_self_update(SIGNER, &records, &definitions, &empty, issued()),
        Err(SelfUpdateRejection::NoChanges)
    );
}

#[test]
fn definition_less_instance_cannot_self_update() {
    let records = [record(SIGNER, None, &all_fields())];
    let definitions = [definition("scout", "Scout")];
    assert_eq!(
        evaluate_self_update(SIGNER, &records, &definitions, &prompt_draft(), issued()),
        Err(SelfUpdateRejection::NoLinkedDefinition)
    );
}

#[test]
fn draft_naming_a_definition_that_is_not_the_signers_own_is_rejected() {
    let records = [record(SIGNER, Some("scout"), &all_fields())];
    let definitions = [definition("scout", "Scout"), definition("pip", "Pip")];
    let draft = SelfUpdateDraft {
        agent_name: "pip".into(),
        ..prompt_draft()
    };
    assert_eq!(
        evaluate_self_update(SIGNER, &records, &definitions, &draft, issued()),
        Err(SelfUpdateRejection::TargetMismatch)
    );
    let missing = SelfUpdateDraft {
        agent_name: "Nobody".into(),
        ..prompt_draft()
    };
    assert_eq!(
        evaluate_self_update(SIGNER, &records, &definitions, &missing, issued()),
        Err(SelfUpdateRejection::TargetMismatch)
    );
}

#[test]
fn ambiguous_display_name_is_rejected() {
    let records = [record(SIGNER, Some("scout"), &all_fields())];
    let definitions = [
        definition("scout", "Scout"),
        definition("scout-2", " scout "),
    ];
    assert_eq!(
        evaluate_self_update(SIGNER, &records, &definitions, &prompt_draft(), issued()),
        Err(SelfUpdateRejection::AmbiguousTarget)
    );
}

#[test]
fn team_sourced_definition_is_not_editable() {
    let records = [record(SIGNER, Some("scout"), &all_fields())];
    let mut team = definition("scout", "Scout");
    team.source_team = Some("team-1".into());
    assert_eq!(
        evaluate_self_update(SIGNER, &records, &[team], &prompt_draft(), issued()),
        Err(SelfUpdateRejection::DefinitionNotEditable)
    );
}

#[test]
fn sibling_sharing_the_definition_must_allow_the_same_field() {
    let definitions = [definition("scout", "Scout")];
    let locked_sibling = [
        record(SIGNER, Some("scout"), &all_fields()),
        record(OTHER, Some("scout"), &[SelfUpdateField::Model]),
    ];
    assert_eq!(
        evaluate_self_update(
            SIGNER,
            &locked_sibling,
            &definitions,
            &prompt_draft(),
            issued()
        ),
        Err(SelfUpdateRejection::SiblingNotAllowed {
            pubkey: OTHER.into(),
            field: "system_prompt",
        })
    );
    let open_sibling = [
        record(SIGNER, Some("scout"), &all_fields()),
        record(OTHER, Some("scout"), &[SelfUpdateField::SystemPrompt]),
    ];
    assert!(evaluate_self_update(
        SIGNER,
        &open_sibling,
        &definitions,
        &prompt_draft(),
        issued()
    )
    .is_ok());
    // An unrelated agent with no policy is not a sibling and does not block.
    let unrelated = [
        record(SIGNER, Some("scout"), &all_fields()),
        record(OTHER, Some("pip"), &[]),
    ];
    assert!(
        evaluate_self_update(SIGNER, &unrelated, &definitions, &prompt_draft(), issued()).is_ok()
    );
}

// ── Replay across a Desktop restart (predates-definition rule) ──────────────

#[test]
fn draft_issued_before_the_definitions_last_write_is_refused() {
    // Scenario: agent applies at T, owner reverts via the form at T+1m,
    // Desktop restarts at T+3m and the relay replays the frame. In-memory
    // dedupe is gone and the frame is inside the age window, so only the
    // definition's own `updated_at` can refuse it.
    let records = [record(SIGNER, Some("scout"), &all_fields())];
    let t = Utc.with_ymd_and_hms(2026, 10, 8, 12, 0, 0).unwrap();
    let mut reverted = definition("scout", "Scout");
    reverted.updated_at = "2026-10-08T12:01:00Z".into(); // owner's revert at T+1m
    assert_eq!(
        evaluate_self_update(SIGNER, &records, &[reverted], &prompt_draft(), t),
        Err(SelfUpdateRejection::PredatesDefinition),
        "a replayed draft must not clobber a later owner edit"
    );

    // The first apply itself bumps `updated_at` to the apply time, so even
    // with no owner edit the replay of the same frame is refused.
    let mut applied = definition("scout", "Scout");
    applied.updated_at = "2026-10-08T12:00:00Z".into(); // apply at exactly T
    assert_eq!(
        evaluate_self_update(SIGNER, &records, &[applied], &prompt_draft(), t),
        Err(SelfUpdateRejection::PredatesDefinition),
        "updated_at equal to issued_at counts as a later write"
    );

    // A draft sent after the last write is the normal case and still passes.
    let mut older = definition("scout", "Scout");
    older.updated_at = "2026-10-08T11:59:59Z".into();
    assert!(evaluate_self_update(SIGNER, &records, &[older], &prompt_draft(), t).is_ok());
}

#[test]
fn unparseable_definition_timestamp_is_treated_as_a_later_write() {
    let records = [record(SIGNER, Some("scout"), &all_fields())];
    let mut odd = definition("scout", "Scout");
    odd.updated_at = String::new();
    assert_eq!(
        evaluate_self_update(SIGNER, &records, &[odd], &prompt_draft(), issued()),
        Err(SelfUpdateRejection::PredatesDefinition)
    );
}

#[test]
fn fresh_draft_issued_at_returns_the_parsed_stamp_only_inside_the_window() {
    let now = Utc.with_ymd_and_hms(2026, 10, 8, 12, 0, 0).unwrap();
    assert_eq!(
        fresh_draft_issued_at(Some("2026-10-08T11:55:00Z"), now),
        Some(Utc.with_ymd_and_hms(2026, 10, 8, 11, 55, 0).unwrap())
    );
    assert_eq!(
        fresh_draft_issued_at(Some("2026-10-08T11:49:59Z"), now),
        None
    );
    assert_eq!(fresh_draft_issued_at(None, now), None);
}

#[test]
fn draft_with_an_unknown_wire_field_does_not_parse() {
    let json = r#"{"channelId":"c","agentName":"Scout","systemPrompt":"x","envVars":{"A":"1"}}"#;
    assert!(serde_json::from_str::<SelfUpdateDraft>(json).is_err());
    let known = r#"{"channelId":"c","agentName":"Scout","systemPrompt":"x"}"#;
    let draft = serde_json::from_str::<SelfUpdateDraft>(known).unwrap();
    assert_eq!(draft.system_prompt.as_deref(), Some("x"));
    assert!(draft.model.is_none());
}

// ── Request projection ───────────────────────────────────────────────────────

#[test]
fn request_carries_untouched_fields_through_and_leaves_env_and_behavior_alone() {
    let persona = definition("scout", "Scout");
    let plan = SelfUpdatePlan {
        persona_id: "scout".into(),
        expected_updated_at: persona.updated_at.clone(),
        display_name: Some("Scout II".into()),
        system_prompt: None,
        model: Some("claude-opus-4".into()),
    };
    let request = self_update_request(&persona, &plan);
    assert_eq!(request.id, "scout");
    assert_eq!(request.display_name, "Scout II");
    assert_eq!(request.system_prompt, "Be helpful.");
    assert_eq!(request.model.as_deref(), Some("claude-opus-4"));
    assert_eq!(request.provider.as_deref(), Some("anthropic"));
    assert_eq!(request.runtime.as_deref(), Some("goose"));
    assert_eq!(request.acp_command.as_deref(), Some("buzz-acp"));
    assert_eq!(
        request.avatar_url.as_deref(),
        Some("https://example.com/a.png")
    );
    assert_eq!(request.description.as_deref(), Some("A helper."));
    assert_eq!(request.name_pool, vec!["Birch".to_string()]);
    assert!(request.env_vars.is_none(), "env vars must not be rewritten");
    assert!(request.behavior.is_none(), "behavior must not be rewritten");
}
