//! Tests for request translation and target resolution.
//!
//! These cover the decisions this module makes on its own — which fields a
//! brokered mutation may touch, and which agent a selector resolves to. The
//! Tauri-calling parts are not covered here: they hold no logic beyond
//! forwarding and the `Failed`/`Unknown` split, and testing them would mean
//! booting an app to observe a call it already delegates.

use buzz_sdk_pkg::broker::AgentTarget;

use super::*;

const CHANNEL: &str = "b2c38ca8-9ec3-411e-bab5-f9deab34d52e";

fn create_args() -> AgentsCreateArgs {
    AgentsCreateArgs {
        channel_id: CHANNEL.into(),
        display_name: "Helper".into(),
        system_prompt: "Help with things.".into(),
        runtime: None,
        provider: None,
        model: None,
        respond_to: None,
    }
}

fn update_args() -> AgentsUpdateArgs {
    AgentsUpdateArgs {
        target: AgentTarget::Name("helper".into()),
        display_name: None,
        system_prompt: None,
        runtime: None,
        provider: None,
        model: None,
        respond_to: None,
    }
}

/// Records are built from JSON, matching `managed_agents::reconcile::tests` —
/// `ManagedAgentRecord` has no `Default`, and enumerating its fifty-odd fields
/// here would break on every unrelated schema addition.
fn record(pubkey: &str, name: &str) -> ManagedAgentRecord {
    serde_json::from_str(&format!(
        r#"{{
            "pubkey": "{pubkey}",
            "name": "{name}",
            "relay_url": "wss://localhost:3000",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "system_prompt": "You are a test agent.",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }}"#
    ))
    .unwrap()
}

/// A remote request for an agent to exist is not a request to run a process on
/// the owner's machine, now or at every launch.
#[test]
fn a_brokered_create_neither_spawns_nor_auto_starts() {
    let request = create_request(&create_args()).unwrap();
    assert!(!request.spawn_after_create);
    assert!(!request.start_on_app_launch);
}

/// Linking a definition would pin someone else's config snapshot onto the new
/// agent — a decision the request never expressed.
#[test]
fn a_brokered_create_links_no_definition_or_team() {
    let request = create_request(&create_args()).unwrap();
    assert!(request.persona_id.is_none());
    assert!(request.team_id.is_none());
    assert!(request.env_vars.is_empty());
    assert_eq!(request.backend, crate::managed_agents::BackendKind::Local);
}

#[test]
fn create_carries_the_requested_name_and_prompt() {
    let request = create_request(&create_args()).unwrap();
    assert_eq!(request.name, "Helper");
    assert_eq!(request.system_prompt.as_deref(), Some("Help with things."));
}

#[test]
fn create_translates_a_respond_to_mode() {
    let mut args = create_args();
    args.respond_to = Some("anyone".into());
    let request = create_request(&args).unwrap();
    assert_eq!(request.respond_to, Some(RespondTo::Anyone));
}

/// An unresolvable runtime must fail here, not later as a spawn error with no
/// connection back to the request that caused it.
#[test]
fn create_refuses_a_runtime_the_catalog_does_not_know() {
    let mut args = create_args();
    args.runtime = Some("not-a-real-runtime".into());
    let error = create_request(&args).unwrap_err();
    assert!(error.contains("unknown runtime"), "unexpected: {error}");
}

/// The broker has no "clear this field" verb, so an unnamed field must stay
/// absent rather than becoming an explicit null that wipes stored config.
#[test]
fn update_leaves_unrequested_fields_untouched() {
    let (request, changed) = update_request("abc", &update_args()).unwrap();
    assert!(changed.is_empty());
    assert!(request.name.is_none());
    assert!(request.model.is_none());
    assert!(request.system_prompt.is_none());
    assert!(request.provider.is_none());
    assert!(request.respond_to.is_none());
    // Fields the capability does not expose at all.
    assert!(request.env_vars.is_none());
    assert!(request.relay_url.is_none());
    assert!(request.agent_args.is_none());
    assert!(request.respond_to_allowlist.is_none());
}

#[test]
fn update_sets_only_the_named_fields_and_reports_them_sorted() {
    let mut args = update_args();
    args.model = Some("some-model".into());
    args.display_name = Some("Renamed".into());
    let (request, changed) = update_request("abc", &args).unwrap();

    assert_eq!(changed, vec!["displayName", "model"]);
    assert_eq!(request.name.as_deref(), Some("Renamed"));
    assert_eq!(request.model, Some(Some("some-model".to_string())));
    assert!(request.system_prompt.is_none());
}

#[test]
fn update_resolves_a_runtime_to_a_command_and_marks_it_a_pin() {
    let mut args = update_args();
    args.runtime = Some("buzz-agent".into());
    let (request, changed) = update_request("abc", &args).unwrap();
    assert_eq!(changed, vec!["runtime"]);
    assert!(request.agent_command.is_some());
    assert!(request.harness_override);
}

#[test]
fn a_pubkey_target_resolves_case_insensitively() {
    let records = vec![record("aabb", "helper")];
    let resolved = resolve_target(&records, &AgentTarget::Pubkey("AABB".into())).unwrap();
    assert_eq!(resolved.pubkey, "aabb");
}

#[test]
fn a_name_target_resolves_case_insensitively() {
    let records = vec![record("aabb", "Helper")];
    let resolved = resolve_target(&records, &AgentTarget::Name("helper".into())).unwrap();
    assert_eq!(resolved.pubkey, "aabb");
}

/// Two agents can legitimately share a name. Picking one would mutate or delete
/// an agent the requester did not identify.
#[test]
fn an_ambiguous_name_is_refused_rather_than_guessed() {
    let records = vec![record("aabb", "helper"), record("ccdd", "Helper")];
    let error = resolve_target(&records, &AgentTarget::Name("helper".into())).unwrap_err();
    assert!(error.contains("more than one"), "unexpected: {error}");
    assert!(error.contains("by pubkey"), "unexpected: {error}");
}

#[test]
fn a_missing_target_is_an_error() {
    let records = vec![record("aabb", "helper")];
    assert!(resolve_target(&records, &AgentTarget::Name("ghost".into())).is_err());
    assert!(resolve_target(&records, &AgentTarget::Pubkey("ffff".into())).is_err());
}
