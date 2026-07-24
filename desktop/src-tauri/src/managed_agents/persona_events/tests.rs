use super::*;
use crate::managed_agents::{BackendKind, ManagedAgentRecord, RespondTo};

/// A linked instance record with no persona-derived fields set yet — the
/// state right after creation, before any snapshot apply.
fn sample_record() -> ManagedAgentRecord {
    ManagedAgentRecord {
        pubkey: "p".repeat(64),
        name: "agent".into(),
        persona_id: Some("test-persona".into()),
        private_key_nsec: "nsec1fake".into(),
        auth_tag: None,
        relay_url: "ws://localhost:3000".into(),
        avatar_url: None,
        acp_command: "buzz-acp".into(),
        agent_command: "goose".into(),
        agent_command_override: None,
        agent_args: vec![],
        mcp_command: String::new(),
        turn_timeout_seconds: 320,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: None,
        env_vars: BTreeMap::new(),
        start_on_app_launch: false,
        auto_restart_on_config_change: true,
        runtime_pid: None,
        backend: BackendKind::Local,
        backend_agent_id: None,
        provider_binary_path: None,
        team_id: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: "now".into(),
        updated_at: "now".into(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to: RespondTo::OwnerOnly,
        respond_to_allowlist: vec![],
        display_name: None,
        slug: None,
        runtime: None,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        definition_respond_to: None,
        definition_respond_to_allowlist: Vec::new(),
        definition_parallelism: None,
        relay_mesh: None,
    }
}

// ── preview_prospective_persona_snapshot (Finding 4: relay-mesh preflight
// ordering) ───────────────────────────────────────────────────────────────

/// Regression for the relay-mesh preflight-ordering defect: the preflight
/// used to check `ensure_relay_mesh_for_record` against the record's stale
/// pre-snapshot bytes, so a persona edit that flips `provider` to
/// `relay-mesh` between saves would not trigger the mesh preflight on the
/// very next start — only on the restart after that, once the real
/// `apply_persona_snapshot` had already landed. The preview must reflect the
/// same prospective re-snapshot the real spawn path applies.
#[test]
fn preview_reflects_persona_edit_flipping_provider_into_relay_mesh() {
    let record = sample_record(); // provider: None (not relay-mesh)
    let mut persona = sample_persona();
    persona.id = "test-persona".into();
    persona.provider = Some("relay-mesh".into());
    persona.model = Some("auto".into());

    let preview = preview_prospective_persona_snapshot(&record, &[persona]);

    assert_eq!(
        preview.provider.as_deref(),
        Some("relay-mesh"),
        "preview must carry the persona's current provider, not the record's stale None"
    );
    assert_eq!(preview.model.as_deref(), Some("auto"));
}

/// Preview reflects a persona edit flipping AWAY from relay-mesh too —
/// symmetric with the into-mesh case above.
#[test]
fn preview_reflects_persona_edit_flipping_provider_out_of_relay_mesh() {
    let mut record = sample_record();
    record.provider = Some("relay-mesh".into());
    record.model = Some("auto".into());
    let mut persona = sample_persona();
    persona.id = "test-persona".into();
    persona.provider = Some("anthropic".into());
    persona.model = Some("claude-opus-4".into());

    let preview = preview_prospective_persona_snapshot(&record, &[persona]);

    assert_eq!(preview.provider.as_deref(), Some("anthropic"));
    assert_eq!(preview.model.as_deref(), Some("claude-opus-4"));
}

/// The preview must not mutate the record passed in — callers (the mesh
/// preflight) need the original bytes intact for the real snapshot apply
/// that follows.
#[test]
fn preview_does_not_mutate_input_record() {
    let record = sample_record();
    let original = record.clone();
    let mut persona = sample_persona();
    persona.id = "test-persona".into();
    persona.provider = Some("relay-mesh".into());

    let _ = preview_prospective_persona_snapshot(&record, &[persona]);

    assert_eq!(record.provider, original.provider);
    assert_eq!(record.model, original.model);
}

/// Orphaned instance (persona deleted): the preview must pass the record
/// through unchanged so downstream orphan handling (refuse to spawn) sees
/// the same bytes it always would.
#[test]
fn preview_passes_through_unchanged_when_persona_missing() {
    let mut record = sample_record();
    record.provider = Some("anthropic".into());
    record.persona_id = Some("deleted-persona".into());

    let preview = preview_prospective_persona_snapshot(&record, &[]);

    assert_eq!(preview.provider.as_deref(), Some("anthropic"));
    assert_eq!(preview.persona_id.as_deref(), Some("deleted-persona"));
}

fn sample_persona() -> AgentDefinition {
    AgentDefinition {
        id: "test-persona".to_string(),
        display_name: "Test Persona".to_string(),
        avatar_url: Some("https://example.com/avatar.png".to_string()),
        system_prompt: "You are a test assistant.".to_string(),
        runtime: Some("goose".to_string()),
        model: Some("claude-opus-4".to_string()),
        provider: Some("anthropic".to_string()),
        name_pool: vec!["Alpha".to_string(), "Beta".to_string()],
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: Some("test-slug".to_string()),
        env_vars: BTreeMap::from([("KEY".to_string(), "value".to_string())]),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "2025-01-01T00:00:00Z".to_string(),
        updated_at: "2025-01-01T00:00:00Z".to_string(),
    }
}

#[test]
fn monotonic_created_at_bumps_past_head() {
    // No head: uses now (floor 0).
    let now = nostr::Timestamp::now().as_secs() as i64;
    let none = monotonic_created_at(None).as_secs() as i64;
    assert!(none >= now, "no-head write must be >= now");

    // Head in the FUTURE (same-second or clock-skewed): must bump to head+1,
    // never reuse now (which would be <= head and lose the NIP-33 tiebreak).
    let future_head = now + 1000;
    let bumped = monotonic_created_at(Some(future_head)).as_secs() as i64;
    assert_eq!(
        bumped,
        future_head + 1,
        "must supersede a future head by +1"
    );

    // Head in the PAST: now already exceeds it, so now wins.
    let past = monotonic_created_at(Some(now - 1000)).as_secs() as i64;
    assert!(past >= now, "past head must not drag created_at backward");
}

#[test]
fn d_tag_uses_slug_when_available() {
    let record = sample_persona();
    assert_eq!(persona_d_tag(&record), "test-slug");
}

#[test]
fn d_tag_falls_back_to_id() {
    let mut record = sample_persona();
    record.source_team_persona_slug = None;
    assert_eq!(persona_d_tag(&record), "test-persona");
}

/// Mirror of the relay slug grammar (`ingest.rs:923` `^[a-z0-9][a-z0-9_-]{0,63}$`)
/// so the normalization tests assert what the relay actually enforces.
fn passes_relay_slug_grammar(d: &str) -> bool {
    let bytes = d.as_bytes();
    !d.is_empty()
        && d.len() <= 64
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes[1..]
            .iter()
            .all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

#[test]
fn d_tag_normalizes_pack_slug_to_relay_grammar() {
    // The cited failing cases: mixed-case and leading-underscore pack slugs
    // that the relay rejects un-normalized → pending forever.
    for (raw, expected) in [
        ("CodeReviewer", "codereviewer"),
        ("_ops", "a_ops"),
        ("Code-Reviewer", "code-reviewer"),
        ("UPPER_snake", "upper_snake"),
        ("-leading-dash", "a-leading-dash"),
    ] {
        let mut record = sample_persona();
        record.source_team_persona_slug = Some(raw.to_string());
        let d = persona_d_tag(&record);
        assert_eq!(d, expected, "normalization of {raw:?}");
        assert!(
            passes_relay_slug_grammar(&d),
            "normalized {raw:?} -> {d:?} still fails the relay grammar"
        );
    }
}

#[test]
fn d_tag_already_valid_slug_is_unchanged() {
    // In-app personas use a lowercase-hex UUID id — already valid, must pass
    // through untouched (no spurious coordinate change on existing data).
    let mut record = sample_persona();
    record.source_team_persona_slug = None;
    record.id = "11111111-2222-3333-4444-555555555555".to_string();
    let d = persona_d_tag(&record);
    assert_eq!(d, "11111111-2222-3333-4444-555555555555");
    assert!(passes_relay_slug_grammar(&d));
}

#[test]
fn build_persona_event_produces_correct_kind() {
    let record = sample_persona();
    let builder = build_persona_event(&record).unwrap();
    let keys = nostr::Keys::generate();
    let event = builder.sign_with_keys(&keys).unwrap();
    assert_eq!(event.kind.as_u16() as u32, KIND_PERSONA);
}

#[test]
fn round_trip_serialization() {
    let record = sample_persona();
    let builder = build_persona_event(&record).unwrap();
    let keys = nostr::Keys::generate();
    let event = builder.sign_with_keys(&keys).unwrap();

    let restored = persona_from_event(&event).unwrap();
    assert_eq!(restored.id, "test-slug");
    assert_eq!(restored.display_name, "Test Persona");
    assert_eq!(
        restored.avatar_url,
        Some("https://example.com/avatar.png".to_string())
    );
    assert_eq!(restored.system_prompt, "You are a test assistant.");
    assert_eq!(restored.runtime, Some("goose".to_string()));
    assert_eq!(restored.model, Some("claude-opus-4".to_string()));
    assert_eq!(restored.provider, Some("anthropic".to_string()));
    assert_eq!(restored.name_pool, vec!["Alpha", "Beta"]);
    // env_vars are not included in public persona events (secrets travel
    // via NIP-44-encrypted engrams only).
    assert!(restored.env_vars.is_empty());
    assert_eq!(
        restored.source_team_persona_slug,
        Some("test-slug".to_string())
    );
    assert!(!restored.is_builtin);
    assert!(restored.is_active);
}

/// NIP-AP reference vector (Event 1, `docs/nips/NIP-AP.md:195-207`): the
/// serialized content bytes MUST match the spec exactly, byte-for-byte.
/// serde emits fields in declaration order, so this pins the content
/// encoding — and therefore the NIP-01 event id — for cross-implementation
/// interop. The field order is `display_name, system_prompt, avatar_url,
/// runtime, model, provider, name_pool`.
#[test]
fn content_matches_nip_ap_vector() {
    // Exact body from NIP-AP.md Event 1 (no trailing whitespace, no BOM).
    const VECTOR: &str = r#"{"display_name":"Test Agent","system_prompt":"You are a test assistant.","avatar_url":"https://example.com/avatar.png","runtime":"goose","model":"claude-opus-4","provider":"anthropic","name_pool":["Alpha","Beta"]}"#;

    let content = PersonaEventContent {
        display_name: "Test Agent".to_string(),
        system_prompt: Some("You are a test assistant.".to_string()),
        avatar_url: Some("https://example.com/avatar.png".to_string()),
        runtime: Some("goose".to_string()),
        model: Some("claude-opus-4".to_string()),
        provider: Some("anthropic".to_string()),
        name_pool: vec!["Alpha".to_string(), "Beta".to_string()],
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
    };
    assert_eq!(
        serde_json::to_string(&content).unwrap(),
        VECTOR,
        "serialized content drifted from the NIP-AP Event 1 vector"
    );

    // Hash invariance across the unified-model widening: REAL pre-revision
    // content bytes (fixture string, not a round-trip through the new
    // struct) must parse and re-serialize byte-identically, so
    // persona_content_hash — the drift-badge basis — is unchanged on
    // upgrade. A bare Option serializing "system_prompt":null would flip
    // every persona's hash fleet-wide.
    let parsed: PersonaEventContent = serde_json::from_str(VECTOR).unwrap();
    assert_eq!(
        serde_json::to_string(&parsed).unwrap(),
        VECTOR,
        "pre-revision content bytes must survive a parse/serialize round-trip unchanged"
    );
    assert_eq!(
        persona_content_hash(&parsed),
        {
            use sha2::{Digest, Sha256};
            hex::encode(Sha256::digest(VECTOR.as_bytes()))
        },
        "persona_content_hash of pre-revision bytes must equal the direct digest"
    );

    // Hash stability, adversarial shapes: the empty prompt and the
    // minimal old-writer body (display_name + system_prompt only) are the
    // two easiest regressions if the projection or skip attributes ever
    // change.
    const EMPTY_PROMPT: &str = r#"{"display_name":"X","system_prompt":""}"#;
    let parsed: PersonaEventContent = serde_json::from_str(EMPTY_PROMPT).unwrap();
    assert_eq!(serde_json::to_string(&parsed).unwrap(), EMPTY_PROMPT);
    const MINIMAL: &str = r#"{"display_name":"Minimal","system_prompt":"Hello."}"#;
    let parsed: PersonaEventContent = serde_json::from_str(MINIMAL).unwrap();
    assert_eq!(serde_json::to_string(&parsed).unwrap(), MINIMAL);

    // An event built from this content carries the byte-exact vector as its
    // signed content, so a second implementer following the spec computes
    // the same NIP-01 id.
    let record = AgentDefinition {
        id: "test-agent".to_string(),
        display_name: "Test Agent".to_string(),
        avatar_url: Some("https://example.com/avatar.png".to_string()),
        system_prompt: "You are a test assistant.".to_string(),
        runtime: Some("goose".to_string()),
        model: Some("claude-opus-4".to_string()),
        provider: Some("anthropic".to_string()),
        name_pool: vec!["Alpha".to_string(), "Beta".to_string()],
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        env_vars: BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "2025-01-01T00:00:00Z".to_string(),
        updated_at: "2025-01-01T00:00:00Z".to_string(),
    };
    let event = build_persona_event(&record)
        .unwrap()
        .sign_with_keys(&nostr::Keys::generate())
        .unwrap();
    assert_eq!(event.content, VECTOR);
}

#[test]
fn round_trip_minimal_persona() {
    let record = AgentDefinition {
        id: "minimal".to_string(),
        display_name: "Minimal".to_string(),
        avatar_url: None,
        system_prompt: "Hello".to_string(),
        runtime: None,
        model: None,
        provider: None,
        name_pool: vec![],
        is_builtin: true,
        is_active: false,
        source_team: Some("team-1".to_string()),
        source_team_persona_slug: None,
        env_vars: BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "2025-01-01T00:00:00Z".to_string(),
        updated_at: "2025-01-01T00:00:00Z".to_string(),
    };

    let builder = build_persona_event(&record).unwrap();
    let keys = nostr::Keys::generate();
    let event = builder.sign_with_keys(&keys).unwrap();

    let restored = persona_from_event(&event).unwrap();
    assert_eq!(restored.id, "minimal");
    assert_eq!(restored.display_name, "Minimal");
    assert_eq!(restored.avatar_url, None);
    assert_eq!(restored.runtime, None);
    assert_eq!(restored.model, None);
    assert_eq!(restored.provider, None);
    assert!(restored.name_pool.is_empty());
    assert!(restored.env_vars.is_empty());
    // Deserialized persona is always non-builtin and active
    assert!(!restored.is_builtin);
    assert!(restored.is_active);
}

#[test]
fn build_persona_delete_has_single_a_tag_no_e_tag() {
    const OWNER: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    let builder = build_persona_delete("test-slug", OWNER).unwrap();
    let keys = nostr::Keys::generate();
    let event = builder.sign_with_keys(&keys).unwrap();

    assert_eq!(event.kind, Kind::Custom(5));

    let a_tags: Vec<&[String]> = event
        .tags
        .iter()
        .map(|t| t.as_slice())
        .filter(|v| v.first().map(String::as_str) == Some("a"))
        .collect();
    assert_eq!(a_tags.len(), 1);
    assert_eq!(a_tags[0][1], format!("{KIND_PERSONA}:{OWNER}:test-slug"));

    // An e-tag would route to the event-id deletion path and leave the
    // replaceable coordinate live — the tombstone must carry none.
    assert!(event
        .tags
        .iter()
        .all(|t| t.as_slice().first().map(String::as_str) != Some("e")));
}

/// NIP-AP behavioral defaults are LIVE since B5 (create-path
/// unification): the wire fields are carried on AgentDefinition in wire
/// shape and re-emitted verbatim by the projection — a foreign
/// definition's behavioral values now survive a local
/// edit-and-republish cycle. This test replaces
/// `behavioral_defaults_are_staged_not_applied` (the staging lock),
/// whose deliberate removal was pinned in the B5 review gates.
#[test]
fn behavioral_defaults_survive_record_round_trip() {
    const FOREIGN: &str = r#"{"display_name":"F","system_prompt":"p","respond_to":"anyone","respond_to_allowlist":["deadbeef"],"parallelism":4}"#;
    let parsed: PersonaEventContent = serde_json::from_str(FOREIGN).unwrap();
    // Wire layer preserves the fields...
    assert_eq!(parsed.respond_to.as_deref(), Some("anyone"));
    assert_eq!(parsed.parallelism, Some(4));
    // ...and the record round-trip now carries them through.
    let record = persona_from_event_content_for_test(parsed);
    let reprojected = persona_event_content(&record);
    assert_eq!(reprojected.respond_to.as_deref(), Some("anyone"));
    assert_eq!(reprojected.respond_to_allowlist, vec!["deadbeef"]);
    assert_eq!(reprojected.parallelism, Some(4));
}

/// B5 hash row 1: a quad-absent definition's content bytes — and
/// therefore `persona_content_hash` — are identical before and after
/// quad activation. Pre-activation the projection hardcoded `None`;
/// post-activation it copies the record's (absent) quad. Both serialize
/// to the same bytes via `skip_serializing_if`, so no drift badge flips
/// and no republish wave fires for quad-absent definitions.
#[test]
fn quad_absent_definition_hash_stable_across_activation() {
    let record = AgentDefinition {
        id: "quad-absent".to_string(),
        display_name: "Test".to_string(),
        avatar_url: None,
        system_prompt: "Hello".to_string(),
        runtime: Some("goose".to_string()),
        model: Some("gpt-oss".to_string()),
        provider: None,
        name_pool: vec!["nib".to_string()],
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        env_vars: BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let live = persona_event_content(&record);
    // The reserved-era projection: identical fields, quad hardcoded off.
    let reserved_era = PersonaEventContent {
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        ..live.clone()
    };
    assert_eq!(
        serde_json::to_string(&live).unwrap(),
        serde_json::to_string(&reserved_era).unwrap(),
        "quad-absent projection must serialize byte-identically to the reserved era"
    );
    assert_eq!(
        persona_content_hash(&live),
        persona_content_hash(&reserved_era)
    );
}

/// Test-only bridge: build an AgentDefinition from parsed content the same
/// way `persona_from_event` maps fields, without needing a signed event.
fn persona_from_event_content_for_test(content: PersonaEventContent) -> AgentDefinition {
    AgentDefinition {
        id: "staged".to_string(),
        display_name: content.display_name,
        avatar_url: content.avatar_url,
        system_prompt: content.system_prompt.unwrap_or_default(),
        runtime: content.runtime,
        model: content.model,
        provider: content.provider,
        name_pool: content.name_pool,
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        env_vars: BTreeMap::new(),
        respond_to: content.respond_to,
        respond_to_allowlist: content.respond_to_allowlist,
        parallelism: content.parallelism,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

#[test]
fn persona_content_hash_is_deterministic() {
    let content = PersonaEventContent {
        display_name: "Test".to_string(),
        avatar_url: None,
        system_prompt: Some("Hello".to_string()),
        runtime: None,
        model: None,
        provider: None,
        name_pool: vec![],
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
    };
    let hash1 = persona_content_hash(&content);
    let hash2 = persona_content_hash(&content);
    assert_eq!(hash1, hash2);
    assert_eq!(hash1.len(), 64); // SHA-256 hex
}

#[test]
fn persona_content_hash_changes_on_edit() {
    let content1 = PersonaEventContent {
        display_name: "Test".to_string(),
        avatar_url: None,
        system_prompt: Some("Hello".to_string()),
        runtime: None,
        model: None,
        provider: None,
        name_pool: vec![],
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
    };
    let mut content2 = content1.clone();
    content2.system_prompt = Some("Goodbye".to_string());
    assert_ne!(
        persona_content_hash(&content1),
        persona_content_hash(&content2)
    );
}

// ── PersonaSnapshot.runtime ───────────────────────────────────────────────

/// (b) The snapshot carries the persona's runtime VERBATIM — including None,
/// which clears a stale materialized value on the instance record. Unlike
/// model/provider, runtime does not fall back to the record's own value:
/// instances have no user-owned runtime, so the definition must stay
/// authoritative.
#[test]
fn snapshot_runtime_verbatim_from_persona() {
    let persona = sample_persona(); // runtime = Some("goose")
    let snap = persona_snapshot(&persona);
    assert_eq!(
        snap.runtime.as_deref(),
        Some("goose"),
        "persona runtime Some must be copied verbatim into snapshot"
    );

    let mut no_runtime = sample_persona();
    no_runtime.runtime = None;
    let snap = persona_snapshot(&no_runtime);
    assert_eq!(
        snap.runtime, None,
        "persona runtime None must produce None snapshot (clears stale materialized value)"
    );
}

// ── persona_snapshot (definition-authoritative) ──────────────────────────

/// Helper: a persona with no model/provider configured.
fn blank_model_persona() -> AgentDefinition {
    AgentDefinition {
        model: None,
        provider: None,
        ..sample_persona()
    }
}

/// Definition-authoritative: blank definition model/provider → snapshot is
/// None. The effective-config resolver handles global fallback at read time.
#[test]
fn blank_definition_clears_model_provider() {
    let persona = blank_model_persona();
    let expected_version = persona_content_hash(&persona_event_content(&persona));

    let snapshot = persona_snapshot(&persona);

    assert!(
        snapshot.model.is_none(),
        "blank definition model must produce None"
    );
    assert!(
        snapshot.provider.is_none(),
        "blank definition provider must produce None"
    );
    assert_eq!(
        snapshot.source_version, expected_version,
        "source_version must still reflect current persona hash"
    );
}

/// Persona has model/provider set → snapshot carries them verbatim.
#[test]
fn snapshot_carries_persona_model_provider() {
    let persona = sample_persona(); // has model=Some("claude-opus-4"), provider=Some("anthropic")

    let snapshot = persona_snapshot(&persona);

    assert_eq!(
        snapshot.model.as_deref(),
        Some("claude-opus-4"),
        "persona model must be carried verbatim"
    );
    assert_eq!(
        snapshot.provider.as_deref(),
        Some("anthropic"),
        "persona provider must be carried verbatim"
    );
}

/// Both blank → snapshot keeps None; a genuinely unconfigured agent
/// stays unconfigured (no fabricated values).
#[test]
fn both_blank_stays_none() {
    let persona = blank_model_persona();

    let snapshot = persona_snapshot(&persona);

    assert!(
        snapshot.model.is_none(),
        "blank persona model → snapshot None"
    );
    assert!(
        snapshot.provider.is_none(),
        "blank persona provider → snapshot None"
    );
}

/// Whitespace-only values on the definition are preserved verbatim in the
/// snapshot. The effective-config resolver treats whitespace as blank.
#[test]
fn whitespace_only_definition_preserved_verbatim() {
    let mut persona = sample_persona();
    persona.model = Some("  ".to_string());
    persona.provider = Some("\t".to_string());

    let snapshot = persona_snapshot(&persona);

    assert_eq!(
        snapshot.model.as_deref(),
        Some("  "),
        "whitespace-only model is preserved verbatim in snapshot"
    );
    assert_eq!(
        snapshot.provider.as_deref(),
        Some("\t"),
        "whitespace-only provider is preserved verbatim in snapshot"
    );
}

/// Cross-field independence: definition sets model but not provider → model
/// comes from definition, provider is None (global fallback at read time).
#[test]
fn definition_model_set_provider_blank_produces_none_provider() {
    let mut persona = sample_persona();
    persona.provider = None;

    let snapshot = persona_snapshot(&persona);

    assert_eq!(
        snapshot.model.as_deref(),
        Some("claude-opus-4"),
        "definition model must be used"
    );
    assert!(
        snapshot.provider.is_none(),
        "definition provider=None → snapshot None"
    );
}

/// Definition provider set, model blank → model=None (global fallback at
/// read time), provider from definition.
#[test]
fn definition_provider_set_model_blank_produces_none_model() {
    let mut persona = sample_persona();
    persona.model = None;

    let snapshot = persona_snapshot(&persona);

    assert!(
        snapshot.model.is_none(),
        "definition model=None → snapshot None"
    );
    assert_eq!(
        snapshot.provider.as_deref(),
        Some("anthropic"),
        "definition provider must be used"
    );
}

// Gated off Windows for the same reason as `archive::real_relay`:
// `build_app_state()` pulls native DLLs unavailable in the Windows CI
// runner. This stub-relay test is hermetic (localhost axum) otherwise.
#[cfg(not(target_os = "windows"))]
mod flush_barrier {
    use super::*;
    use crate::app_state::build_app_state;
    use crate::managed_agents::retention::{
        get_retained_event, open_retention_db, retain_event, tombstone_retention_d_tag,
        RetainedEvent,
    };
    use nostr::JsonUtil;

    /// Stub relay: `POST /events` rejects kind:5 with HTTP 500, accepts
    /// everything else. Returns the HTTP base URL.
    async fn spawn_stub_relay() -> String {
        use axum::{http::StatusCode, routing::post, Router};

        let app = Router::new().route(
            "/events",
            post(|body: String| async move {
                let event: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                if event.get("kind").and_then(serde_json::Value::as_u64) == Some(5) {
                    return (StatusCode::INTERNAL_SERVER_ERROR, String::new());
                }
                (
                    StatusCode::OK,
                    serde_json::json!({
                        "event_id": event.get("id").and_then(serde_json::Value::as_str).unwrap_or(""),
                        "accepted": true,
                        "message": ""
                    })
                    .to_string(),
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind stub relay");
        let addr = listener.local_addr().expect("stub relay addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        format!("http://{addr}")
    }

    fn retain_signed(
        conn: &rusqlite::Connection,
        keys: &nostr::Keys,
        kind: u32,
        retention_d_tag: &str,
        builder: nostr::EventBuilder,
        created_at: i64,
    ) {
        let event = builder.sign_with_keys(keys).expect("sign test event");
        retain_event(
            conn,
            &RetainedEvent {
                kind,
                pubkey: keys.public_key().to_hex(),
                d_tag: retention_d_tag.to_string(),
                content: event.content.to_string(),
                created_at,
                raw_event: event.as_json(),
                pending_sync: true,
            },
        )
        .expect("retain test event");
    }

    #[test]
    fn archive_request_resign_refreshes_timestamp_and_preserves_payload() {
        use nostr::JsonUtil;

        let keys = nostr::Keys::generate();
        let target = nostr::Keys::generate().public_key().to_hex();
        let stale = crate::events::build_archive_identity_request(
            &target,
            "agent deleted",
            Some("retired"),
            None,
            None,
        )
        .unwrap()
        .custom_created_at(nostr::Timestamp::from(1))
        .sign_with_keys(&keys)
        .unwrap();
        let state = build_app_state();
        *state.keys.lock().unwrap() = keys;

        let fresh = resign_with_fresh_timestamp(&stale, &state).unwrap();

        assert!(fresh.created_at.as_secs() > stale.created_at.as_secs());
        assert_eq!(fresh.kind, stale.kind);
        assert_eq!(fresh.content, stale.content);
        assert_eq!(fresh.tags, stale.tags);
        assert!(fresh.verify_id());
        assert!(fresh.verify_signature());
        assert_ne!(fresh.as_json(), stale.as_json());
    }

    /// The mid-sweep barrier: a tombstone the relay rejects must defer its
    /// own replacement to the next sweep (still pending, not counted as
    /// flushed) while unrelated rows in the same sweep publish normally.
    /// Failing toward stay-deleted is the safe direction — the deferred
    /// replacement can never be wiped by its own late tombstone.
    #[tokio::test]
    async fn failed_tombstone_defers_replacement_within_sweep() {
        let keys = nostr::Keys::generate();
        let pubkey = keys.public_key().to_hex();
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("retention.db");

        {
            let conn = open_retention_db(&db_path).expect("open db");
            // Tombstone (publishes first, relay rejects it).
            retain_signed(
                &conn,
                &keys,
                5,
                &tombstone_retention_d_tag(KIND_PERSONA, "covered"),
                build_persona_delete("covered", &pubkey).unwrap(),
                1000,
            );
            // Its replacement at the same coordinate (must defer).
            retain_signed(
                &conn,
                &keys,
                KIND_PERSONA,
                "covered",
                EventBuilder::new(Kind::Custom(KIND_PERSONA as u16), "{}")
                    .tags(vec![Tag::parse(["d", "covered"]).unwrap()]),
                2000,
            );
            // Unrelated coordinate (must publish despite the barrier).
            retain_signed(
                &conn,
                &keys,
                KIND_PERSONA,
                "unrelated",
                EventBuilder::new(Kind::Custom(KIND_PERSONA as u16), "{}")
                    .tags(vec![Tag::parse(["d", "unrelated"]).unwrap()]),
                1500,
            );
        }

        let state = build_app_state();
        *state.relay_url_override.lock().unwrap() = Some(spawn_stub_relay().await);

        let flushed = flush_pending_events(&db_path, &state).await.expect("flush");
        assert_eq!(flushed, 1, "only the unrelated row publishes");

        let conn = open_retention_db(&db_path).expect("reopen db");
        let row = |kind: u32, d_tag: &str| {
            get_retained_event(&conn, kind, &pubkey, d_tag)
                .unwrap()
                .unwrap()
        };
        assert!(
            row(5, &tombstone_retention_d_tag(KIND_PERSONA, "covered")).pending_sync,
            "failed tombstone stays pending"
        );
        assert!(
            row(KIND_PERSONA, "covered").pending_sync,
            "deferred replacement stays pending"
        );
        assert!(
            !row(KIND_PERSONA, "unrelated").pending_sync,
            "unrelated row marked synced"
        );
    }
}
