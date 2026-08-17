//! Runtime-command status tests: relay-pin fan-out, and the fail-closed
//! `local_setup` gates (global/definition/harness tiers).
//!
//! Extracted from `runtime_commands.rs` via `#[path]` so that module stays
//! under the desktop file-size ratchet; `super::*` resolves against it,
//! matching the `readiness_goose_file_config_tests.rs` convention.

use super::*;

fn payload(
    relay_url: &str,
    lifecycle: ManagedAgentRuntimeLifecycle,
    error: Option<&str>,
) -> super::super::ManagedAgentRuntimeLifecycleObserverPayload {
    super::super::ManagedAgentRuntimeLifecycleObserverPayload {
        pubkey: "aa".repeat(32),
        relay_url: relay_url.into(),
        start_nonce: "test-generation".into(),
        lifecycle,
        error: error.map(str::to_owned),
    }
}

fn record_with_relay(relay_url: &str) -> super::super::ManagedAgentRecord {
    serde_json::from_str(&format!(
        r#"{{
                "pubkey": "{}",
                "name": "pin-test",
                "relay_url": "{relay_url}",
                "acp_command": "buzz-acp",
                "agent_command": "goose",
                "agent_args": [],
                "mcp_command": "",
                "turn_timeout_seconds": 320,
                "system_prompt": "",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z"
            }}"#,
        "aa".repeat(32)
    ))
    .unwrap()
}

#[test]
fn legacy_relay_pin_is_ignored_for_fan_out() {
    // Zero-touch cutover (#2122): a record carrying a creation-era
    // `relay_url` pin must fan out exactly like an unpinned one — the
    // stored field is parsed but never consulted. See
    // `effective_agent_relay_url`.
    let unpinned = record_with_relay("");
    let pinned = record_with_relay("wss://one.example");
    for record in [&unpinned, &pinned] {
        assert_eq!(
            crate::relay::effective_agent_relay_url(&record.relay_url, "wss://two.example"),
            "wss://two.example"
        );
    }
}

#[test]
fn unkeyable_relay_degrades_to_failed_row() {
    // A requested URL that cannot form a pair key must still yield a
    // Failed row keyed by the raw requested string, so one bad community
    // never aborts the rest of the reconcile batch.
    let record = record_with_relay("");
    let status = unkeyable_failed_status(
        &record,
        "not a url".to_string(),
        "relay access probe timed out".to_string(),
        &[],
        &super::super::GlobalAgentConfig::default(),
        false,
    );
    assert!(matches!(
        status.lifecycle,
        ManagedAgentRuntimeLifecycle::Failed
    ));
    assert_eq!(status.relay_url, "not a url");
    assert_eq!(status.requested_relay_url.as_deref(), Some("not a url"));
    assert_eq!(status.pubkey, record.pubkey);
    assert_eq!(
        status.error.as_deref(),
        Some("relay access probe timed out")
    );
    assert!(status.pid.is_none());
}

#[test]
fn runtime_key_rejects_non_hex_pubkeys() {
    assert!(ManagedAgentRuntimeKey::new("../not-a-key", "wss://relay.example").is_err());
    assert!(ManagedAgentRuntimeKey::new("gg".repeat(32), "wss://relay.example").is_err());
}

#[test]
fn runtime_key_canonicalizes_hex_pubkeys() {
    let key = ManagedAgentRuntimeKey::new("AA".repeat(32), "wss://relay.example").unwrap();
    assert_eq!(key.pubkey, "aa".repeat(32));
}

#[test]
fn observer_lifecycle_key_preserves_exact_canonical_pair() {
    let first = payload(
        "WSS://Relay.Example:443/",
        ManagedAgentRuntimeLifecycle::Ready,
        None,
    );
    let key = observer_lifecycle_key(&first.pubkey, &first).unwrap();
    assert_eq!(key.pubkey, first.pubkey);
    assert_eq!(key.relay_url, "wss://relay.example");

    let other = payload(
        "wss://other.example",
        ManagedAgentRuntimeLifecycle::Ready,
        None,
    );
    assert_ne!(key, observer_lifecycle_key(&other.pubkey, &other).unwrap());
}

#[test]
fn observer_lifecycle_rejects_cross_agent_and_desktop_states() {
    let ready = payload(
        "wss://relay.example",
        ManagedAgentRuntimeLifecycle::Ready,
        None,
    );
    assert!(observer_lifecycle_key(&"bb".repeat(32), &ready).is_err());

    let stopped = payload(
        "wss://relay.example",
        ManagedAgentRuntimeLifecycle::Stopped,
        None,
    );
    assert!(observer_lifecycle_key(&stopped.pubkey, &stopped).is_err());
}

#[test]
fn observer_lifecycle_enforces_failed_error_contract() {
    let failed = payload(
        "wss://relay.example",
        ManagedAgentRuntimeLifecycle::Failed,
        None,
    );
    assert!(observer_lifecycle_key(&failed.pubkey, &failed).is_err());

    let ready_with_error = payload(
        "wss://relay.example",
        ManagedAgentRuntimeLifecycle::Ready,
        Some("unexpected"),
    );
    assert!(observer_lifecycle_key(&ready_with_error.pubkey, &ready_with_error).is_err());
}

#[test]
fn secrets_unavailable_forces_local_setup_false() {
    // When `secrets_unavailable` is true on a record, `local_setup` must
    // be false regardless of what `agent_readiness` would return for the
    // effective env — spawn will refuse, and the UI must reflect that.
    let mut record = record_with_relay("");
    record.secrets_unavailable = true;

    let status = unkeyable_failed_status(
        &record,
        "wss://relay.example".to_string(),
        "test error".to_string(),
        &[],
        &super::super::GlobalAgentConfig::default(),
        false,
    );
    assert!(
        !status.local_setup,
        "local_setup must be false when secrets_unavailable is true"
    );
}

#[test]
fn global_unavailable_forces_local_setup_false() {
    // When the global env_vars ref cannot be resolved from the keyring,
    // spawn refuses in `spawn_agent_child`. A record with fully-available
    // per-agent secrets must still read not-ready so the UI never
    // advertises a local_setup the agent cannot honor.
    let record = record_with_relay("");
    assert!(
        !record.secrets_unavailable,
        "guard: this test isolates the global-tier gate, not the record gate"
    );

    let status = unkeyable_failed_status(
        &record,
        "wss://relay.example".to_string(),
        "test error".to_string(),
        &[],
        &super::super::GlobalAgentConfig::default(),
        true, // global_unavailable
    );
    assert!(
        !status.local_setup,
        "local_setup must be false when the global config is unavailable"
    );
}

fn linked_record() -> super::super::ManagedAgentRecord {
    let mut rec = record_with_relay("");
    rec.persona_id = Some("def-slug".to_string());
    rec
}

/// A linked record whose effective env satisfies the `buzz-agent` readiness
/// gate, so `agent_readiness` returns `Ready`. This makes the linked
/// definition's `secrets_unavailable` flag the SOLE remaining input to
/// `local_setup` — without a Ready baseline, `local_setup` is false for
/// unrelated reasons (empty env) and the definition gate cannot be observed.
/// The command resolves to `buzz-agent` (persona runtime absent →
/// `default_agent_command`), and these three non-reserved env vars survive
/// into the effective env via the record's own `env_vars` layer.
fn ready_linked_record() -> super::super::ManagedAgentRecord {
    let mut rec = linked_record();
    rec.env_vars
        .insert("BUZZ_AGENT_PROVIDER".into(), "anthropic".into());
    rec.env_vars
        .insert("BUZZ_AGENT_MODEL".into(), "claude-opus-4-5".into());
    rec.env_vars
        .insert("ANTHROPIC_API_KEY".into(), "sk-test".into());
    rec
}

fn available_definition() -> super::super::AgentDefinition {
    let mut d = super::super::AgentDefinition {
        id: "def-slug".to_string(),
        display_name: "Def".to_string(),
        system_prompt: "prompt".to_string(),
        created_at: "2026-01-01".to_string(),
        updated_at: "2026-01-01".to_string(),
        ..Default::default()
    };
    d.secrets_unavailable = false;
    d
}

fn unavailable_definition() -> super::super::AgentDefinition {
    let mut d = available_definition();
    d.secrets_unavailable = true;
    d
}

/// Positive control for the two tests below: with a Ready baseline and an
/// AVAILABLE definition, `local_setup` is true. This is what makes the
/// negative assertions meaningful — it proves the `false` outcomes there
/// are caused by the definition-unavailable gate, not by the record failing
/// readiness for some other reason. If the readiness recipe ever drifts and
/// this record stops being Ready, this test fails first and loudly.
#[test]
fn ready_linked_record_with_available_definition_is_local_setup_true() {
    let record = ready_linked_record();
    let def = available_definition();

    let status = unkeyable_failed_status(
        &record,
        "wss://relay.example".to_string(),
        "test error".to_string(),
        std::slice::from_ref(&def),
        &super::super::GlobalAgentConfig::default(),
        false,
    );
    assert!(
        status.local_setup,
        "a Ready record linked to an available definition must show local_setup=true"
    );
}

#[test]
fn definition_unavailable_forces_local_setup_false() {
    // A linked instance whose definition's env_vars ref could not be
    // hydrated must show local_setup=false — spawn will refuse, so
    // advertising readiness would promise a launch we cannot honor. The
    // record is otherwise Ready (see the positive control above), so the
    // only thing forcing false here is the definition-unavailable gate:
    // remove `&& !definition_unavailable` from `local_setup` and this fails.
    let record = ready_linked_record();
    let def = unavailable_definition();

    let status = unkeyable_failed_status(
        &record,
        "wss://relay.example".to_string(),
        "test error".to_string(),
        std::slice::from_ref(&def),
        &super::super::GlobalAgentConfig::default(),
        false,
    );
    assert!(
        !status.local_setup,
        "local_setup must be false when the linked definition is unavailable"
    );
}

#[test]
fn unlinked_instance_ignores_definition_unavailability() {
    // An agent with no persona_id is not linked to any definition; an
    // unavailable definition in the slice must not force its local_setup
    // false. The record is Ready, so local_setup stays true — proving the
    // gate keys on persona_id and does not blindly scan the slice. Break
    // the predicate to match any unavailable definition and this fails.
    let mut record = ready_linked_record();
    record.persona_id = None;

    let def = unavailable_definition(); // must be invisible to the unlinked record
    let status = unkeyable_failed_status(
        &record,
        "wss://relay.example".to_string(),
        "test error".to_string(),
        std::slice::from_ref(&def),
        &super::super::GlobalAgentConfig::default(),
        false,
    );
    assert!(
        status.local_setup,
        "an unlinked record must ignore an unavailable definition and stay Ready"
    );
}

// ── Harness-tier gate: an unavailable harness env forces local_setup=false ──
//
// These mirror the definition-tier trio above but exercise the fourth tier:
// the effective harness's own `env_ref` could not be hydrated
// (`env_unavailable`), so `spawn_agent_child` refuses and the status row
// must not advertise a launch it cannot honor. The harness is resolved from
// the in-process registry, so each test holds `registry_test_lock()` and
// restores an empty registry on the way out.

use crate::managed_agents::custom_harnesses::{
    registry_test_lock, update_loaded_harness_registry, HarnessDefinition,
};

/// A custom harness whose command is the always-known `buzz-agent` runtime,
/// so readiness evaluates the same recipe `ready_harness_record` satisfies.
/// `env_unavailable` is the single toggled input across the tests below.
fn harness_def(env_unavailable: bool) -> HarnessDefinition {
    HarnessDefinition {
        id: "my-harness".to_string(),
        label: "My Harness".to_string(),
        command: "buzz-agent".to_string(),
        args: vec![],
        env: std::collections::BTreeMap::new(),
        env_ref: env_unavailable.then(|| "gen-1".to_string()),
        env_unavailable,
        install_instructions_url: String::new(),
        install_hint: String::new(),
    }
}

/// A record pinned to the `my-harness` runtime whose own env vars satisfy
/// the buzz-agent readiness gate — so the harness-unavailable gate is the
/// sole remaining input to `local_setup`.
fn ready_harness_record() -> super::super::ManagedAgentRecord {
    let mut rec = record_with_relay("");
    rec.runtime = Some("my-harness".to_string());
    rec.env_vars
        .insert("BUZZ_AGENT_PROVIDER".into(), "anthropic".into());
    rec.env_vars
        .insert("BUZZ_AGENT_MODEL".into(), "claude-opus-4-5".into());
    rec.env_vars
        .insert("ANTHROPIC_API_KEY".into(), "sk-test".into());
    rec
}

#[test]
fn ready_record_with_available_harness_is_local_setup_true() {
    // Positive control: a Ready record whose pinned harness is AVAILABLE
    // must show local_setup=true — proving the `false` outcome below comes
    // from the harness gate, not from the record failing readiness.
    let _lock = registry_test_lock();
    update_loaded_harness_registry(vec![harness_def(false)]);

    let status = unkeyable_failed_status(
        &ready_harness_record(),
        "wss://relay.example".to_string(),
        "test error".to_string(),
        &[],
        &super::super::GlobalAgentConfig::default(),
        false,
    );

    update_loaded_harness_registry(vec![]);
    assert!(
        status.local_setup,
        "a Ready record on an available harness must show local_setup=true"
    );
}

#[test]
fn harness_unavailable_forces_local_setup_false() {
    // A record whose effective harness carries an unhydratable env_ref must
    // show local_setup=false — spawn refuses via `unavailable_harness_id`.
    // The record is otherwise Ready (see the positive control above), so the
    // only thing forcing false here is the harness gate: remove
    // `&& !harness_unavailable` from `local_setup` and this fails.
    let _lock = registry_test_lock();
    update_loaded_harness_registry(vec![harness_def(true)]);

    let status = unkeyable_failed_status(
        &ready_harness_record(),
        "wss://relay.example".to_string(),
        "test error".to_string(),
        &[],
        &super::super::GlobalAgentConfig::default(),
        false,
    );

    update_loaded_harness_registry(vec![]);
    assert!(
        !status.local_setup,
        "local_setup must be false when the effective harness env is unavailable"
    );
}

#[test]
fn record_on_a_different_harness_ignores_unavailable_one() {
    // A record pinned to a DIFFERENT (available) runtime must not be forced
    // false by an unavailable harness elsewhere in the registry — the gate
    // consults only the harness a spawn would actually launch.
    let _lock = registry_test_lock();
    update_loaded_harness_registry(vec![harness_def(true)]);

    let mut record = ready_harness_record();
    record.runtime = Some("buzz-agent".to_string()); // builtin, always available

    let status = unkeyable_failed_status(
        &record,
        "wss://relay.example".to_string(),
        "test error".to_string(),
        &[],
        &super::super::GlobalAgentConfig::default(),
        false,
    );

    update_loaded_harness_registry(vec![]);
    assert!(
        status.local_setup,
        "a record on a different, available harness must stay Ready"
    );
}

#[test]
fn harness_unavailable_spawn_predicate_fires() {
    // The direct spawn binding (mirrors `definition_unavailable_spawn_predicate_fires`):
    // `spawn_agent_child` gates on `unavailable_harness_id(record, personas)`
    // before any AppHandle-dependent path, so this asserts the exact predicate
    // that gate consults fires for a spawn-shaped record. Deleting the harness
    // env_unavailable set or the predicate filter makes this fail.
    let _lock = registry_test_lock();
    update_loaded_harness_registry(vec![harness_def(true)]);

    let fired = crate::managed_agents::unavailable_harness_id(&ready_harness_record(), &[]);

    update_loaded_harness_registry(vec![]);
    assert_eq!(
        fired.as_deref(),
        Some("my-harness"),
        "spawn must detect the unavailable harness and name it via the shared predicate"
    );
}

#[test]
fn harness_available_spawn_predicate_does_not_fire() {
    // Inverse of the above: an available harness must not fire the spawn gate.
    let _lock = registry_test_lock();
    update_loaded_harness_registry(vec![harness_def(false)]);

    let fired = crate::managed_agents::unavailable_harness_id(&ready_harness_record(), &[]);

    update_loaded_harness_registry(vec![]);
    assert_eq!(
        fired, None,
        "spawn must not refuse a record whose effective harness is available"
    );
}

// ── effective_secrets_unavailable: the restart-eligibility predicate ─────────
//
// The restart-eligibility boundaries (post-install bounce, global-config
// bounce) gate on this single predicate, which ORs the three
// registry-derivable tiers. These tests prove each tier flips it independently
// and a fully-healthy record does not, so the restart flows fail closed on ANY
// unavailable tier — not just the harness one.

use crate::managed_agents::effective_secrets_unavailable;

#[test]
fn effective_secrets_unavailable_false_for_healthy_record() {
    // Positive control: a linked record with an available definition and no
    // harness pin (builtin) has no unavailable tier — the predicate is false,
    // so the negative-tier assertions below are meaningful.
    let _lock = registry_test_lock();
    update_loaded_harness_registry(vec![]);

    let record = linked_record();
    let def = available_definition();

    let out = effective_secrets_unavailable(&record, std::slice::from_ref(&def));

    assert!(
        !out,
        "a record with all tiers available must not be flagged unavailable"
    );
}

#[test]
fn effective_secrets_unavailable_true_for_instance_tier() {
    // Instance tier: the record's own secrets_unavailable set (its env_vars_ref
    // failed to hydrate). Mutation check: drop `record.secrets_unavailable`
    // from the OR and this fails.
    let _lock = registry_test_lock();
    update_loaded_harness_registry(vec![]);

    let mut record = linked_record();
    record.secrets_unavailable = true;
    let def = available_definition();

    assert!(
        effective_secrets_unavailable(&record, std::slice::from_ref(&def)),
        "an instance whose own secrets are unavailable must fire the predicate"
    );
}

#[test]
fn effective_secrets_unavailable_true_for_definition_tier() {
    // Definition tier: a linked persona whose secrets_unavailable is set.
    // Mutation check: drop the `unavailable_definition_id(..).is_some()` arm.
    let _lock = registry_test_lock();
    update_loaded_harness_registry(vec![]);

    let record = linked_record();
    let def = unavailable_definition();

    assert!(
        effective_secrets_unavailable(&record, std::slice::from_ref(&def)),
        "a record linked to a definition with unavailable secrets must fire the predicate"
    );
}

#[test]
fn effective_secrets_unavailable_true_for_harness_tier() {
    // Harness tier: the effective harness carries an unhydratable env_ref.
    // Mutation check: drop the `unavailable_harness_id(..).is_some()` arm.
    let _lock = registry_test_lock();
    update_loaded_harness_registry(vec![harness_def(true)]);

    let out = effective_secrets_unavailable(&ready_harness_record(), &[]);

    update_loaded_harness_registry(vec![]);
    assert!(
        out,
        "a record whose effective harness env is unavailable must fire the predicate"
    );
}
