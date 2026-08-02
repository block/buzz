//! Tests for durable agent-roster enumeration.
//!
//! The JSON in `EXAMPLE_ROSTER` mirrors the shape returned by
//! `openclaw agents list --json`: a primary with no display name plus named
//! agents carrying optional identity metadata.

use super::*;

/// Verbatim excerpt of a real OpenClaw 2026.7.1 roster: a primary with no name,
/// plus named agents carrying identity metadata.
const EXAMPLE_ROSTER: &str = r#"[
  {
    "id": "main",
    "workspace": "/home/example/.openclaw/workspace",
    "agentDir": "/home/example/.openclaw/agents/main/agent",
    "model": "provider/model-primary",
    "bindings": 0,
    "isDefault": true
  },
  {
    "id": "worker-alpha",
    "name": "Worker Alpha",
    "identityName": "Worker Alpha",
    "identitySource": "identity",
    "workspace": "/home/example/.openclaw/agents/worker-alpha/workspace",
    "model": "provider/model-worker",
    "bindings": 0,
    "isDefault": false
  },
  {
    "id": "worker-beta",
    "name": "Worker Beta",
    "identityName": "Worker Beta",
    "workspace": "/home/example/.openclaw/agents/worker-beta/workspace",
    "model": "provider/model-worker",
    "bindings": 1,
    "isDefault": false
  }
]"#;

#[test]
fn parses_a_real_openclaw_roster() {
    let candidates = parse_openclaw_roster(EXAMPLE_ROSTER).expect("roster parses");

    assert_eq!(candidates.len(), 3);
    // Primary first, then alphabetical — the order the selector renders.
    assert_eq!(
        candidates
            .iter()
            .map(|c| c.agent_id.as_str())
            .collect::<Vec<_>>(),
        vec!["main", "worker-alpha", "worker-beta"]
    );
    assert!(candidates[0].is_primary);
    assert!(!candidates[1].is_primary);
    assert!(!candidates[2].is_primary);
    assert_eq!(
        candidates.iter().filter(|c| c.is_primary).count(),
        1,
        "exactly one candidate is preselected"
    );
    assert!(candidates.iter().all(|c| c.harness_id == "openclaw"));
}

#[test]
fn a_primary_with_no_name_falls_back_to_its_id() {
    let candidates = parse_openclaw_roster(EXAMPLE_ROSTER).expect("roster parses");
    let main = &candidates[0];
    assert_eq!(main.agent_id, "main");
    assert_eq!(main.display_name, "main");
}

#[test]
fn identity_name_wins_over_name() {
    let payload = r#"[{"id":"a","name":"Config Name","identityName":"Identity Name"}]"#;
    let candidates = parse_openclaw_roster(payload).expect("parses");
    assert_eq!(candidates[0].display_name, "Identity Name");
}

#[test]
fn name_is_used_when_identity_name_is_absent() {
    let payload = r#"[{"id":"a","name":"Config Name"}]"#;
    let candidates = parse_openclaw_roster(payload).expect("parses");
    assert_eq!(candidates[0].display_name, "Config Name");
}

#[test]
fn blank_names_fall_back_rather_than_rendering_empty() {
    let payload = r#"[{"id":"steve","name":"   ","identityName":""}]"#;
    let candidates = parse_openclaw_roster(payload).expect("parses");
    assert_eq!(candidates[0].display_name, "steve");
}

#[test]
fn main_is_primary_when_the_harness_flags_nothing() {
    // A harness that reports no default must still yield a preselection, or the
    // dialog pushes a choice onto a user with no basis for making it.
    let payload = r#"[{"id":"worker-alpha","name":"Worker Alpha"},{"id":"main"}]"#;
    let candidates = parse_openclaw_roster(payload).expect("parses");
    let main = candidates
        .iter()
        .find(|c| c.agent_id == "main")
        .expect("main present");
    assert!(main.is_primary);
    assert_eq!(candidates[0].agent_id, "main", "primary sorts first");
}

#[test]
fn no_primary_is_claimed_when_there_is_no_default_and_no_main() {
    // Inventing one would be worse than none: the user would enroll an agent
    // Buzz guessed at.
    let payload = r#"[{"id":"worker-alpha","name":"Worker Alpha"},{"id":"worker-beta","name":"Worker Beta"}]"#;
    let candidates = parse_openclaw_roster(payload).expect("parses");
    assert!(candidates.iter().all(|c| !c.is_primary));
}

#[test]
fn an_explicit_default_beats_the_main_fallback() {
    let payload = r#"[{"id":"main"},{"id":"worker-alpha","isDefault":true}]"#;
    let candidates = parse_openclaw_roster(payload).expect("parses");
    assert_eq!(candidates[0].agent_id, "worker-alpha");
    assert!(candidates[0].is_primary);
    let main = candidates
        .iter()
        .find(|c| c.agent_id == "main")
        .expect("main present");
    assert!(!main.is_primary, "the fallback must not double-mark");
}

#[test]
fn duplicate_ids_collapse() {
    let payload = r#"[{"id":"main"},{"id":"main","name":"Second"}]"#;
    let candidates = parse_openclaw_roster(payload).expect("parses");
    assert_eq!(candidates.len(), 1);
}

#[test]
fn rows_without_a_routable_id_are_dropped() {
    let payload = r#"[{"id":"  "},{"id":"worker-alpha","name":"Worker Alpha"}]"#;
    let candidates = parse_openclaw_roster(payload).expect("parses");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].agent_id, "worker-alpha");
}

#[test]
fn blank_optional_details_become_none_rather_than_empty_strings() {
    let payload = r#"[{"id":"worker-alpha","model":"","workspace":"   "}]"#;
    let candidates = parse_openclaw_roster(payload).expect("parses");
    assert_eq!(candidates[0].model, None);
    assert_eq!(candidates[0].workspace, None);
}

#[test]
fn binding_count_is_carried_through() {
    let candidates = parse_openclaw_roster(EXAMPLE_ROSTER).expect("parses");
    let worker = candidates
        .iter()
        .find(|c| c.agent_id == "worker-beta")
        .expect("worker present");
    assert_eq!(worker.binding_count, Some(1));
}

#[test]
fn an_empty_roster_is_an_error_not_an_empty_list() {
    // "This harness has no agents" and "the query returned nothing useful" must
    // not render identically.
    let error = parse_openclaw_roster("[]").expect_err("empty roster rejected");
    assert!(error.contains("no configured agents"), "{error}");
}

#[test]
fn empty_output_is_rejected() {
    let error = parse_openclaw_roster("").expect_err("empty payload rejected");
    assert!(error.contains("no roster output"), "{error}");
}

#[test]
fn malformed_json_is_reported_as_a_parse_failure() {
    let error = parse_openclaw_roster("not json").expect_err("malformed rejected");
    assert!(error.contains("could not parse"), "{error}");
}

#[test]
fn unknown_harness_fields_do_not_break_parsing() {
    // The harness is free to add fields; Buzz names only what it uses.
    let payload = r#"[{"id":"main","isDefault":true,"somethingNew":{"nested":1}}]"#;
    let candidates = parse_openclaw_roster(payload).expect("parses");
    assert_eq!(candidates.len(), 1);
}

#[test]
fn the_remote_command_is_bounded_by_both_markers() {
    let recipe = recipe_for("openclaw").expect("openclaw recipe exists");
    let command = build_roster_command(recipe);
    assert!(command.contains(ROSTER_START));
    assert!(command.contains(ROSTER_END));
    assert!(command.contains("openclaw agents list --json"));
}

#[test]
fn the_remote_command_uses_a_login_but_not_interactive_shell() {
    // `-lic` hangs on a real zsh host with prompt plugins; `-lc` still resolves
    // the login PATH. The host probe was fixed for this and the roster query
    // must not reintroduce it.
    let recipe = recipe_for("openclaw").expect("openclaw recipe exists");
    let command = build_roster_command(recipe);
    assert!(command.contains("$SHELL -lc"), "{command}");
    assert!(!command.contains("-lic"), "{command}");
}

#[test]
fn every_recipe_is_safe_inside_single_quotes() {
    assert!(recipes_are_quote_safe());
}

#[test]
fn the_assembled_command_has_exactly_one_quoted_region() {
    // Regression: the first version emitted markers with `printf '%s\n'`, whose
    // quotes closed the outer quoting early and handed the rest of the command
    // to the shell as code. Asserting the markers were merely *present* passed
    // happily. Two quotes total — the wrapper pair — is the invariant.
    for recipe in ROSTER_RECIPES {
        let command = build_roster_command(recipe);
        assert_eq!(
            command.matches('\'').count(),
            2,
            "command must contain only the wrapping quote pair: {command}"
        );
        let opened = command.find('\'').expect("opening quote");
        let closed = command.rfind('\'').expect("closing quote");
        assert_eq!(closed, command.len() - 1, "quoting must close at the end");
        assert!(opened < closed);
    }
}

#[test]
fn the_assembled_command_is_accepted_by_a_real_shell() {
    // Parse-check the exact string that reaches the remote shell. A quoting bug
    // is otherwise invisible until it runs on someone's host, where it surfaces
    // as an unexplained empty roster.
    for recipe in ROSTER_RECIPES {
        let command = build_roster_command(recipe);
        let status = std::process::Command::new("/bin/sh")
            .arg("-n")
            .arg("-c")
            .arg(&command)
            .status()
            .expect("sh runs");
        assert!(status.success(), "shell rejected: {command}");
    }
}

#[test]
fn payload_extraction_discards_login_shell_noise() {
    let stdout = format!(
        "Welcome to the machine\nLast login: whenever\n{ROSTER_START}\n[]\n{ROSTER_END}\nbye\n"
    );
    assert_eq!(extract_payload(&stdout), Some("[]"));
}

#[test]
fn a_missing_closing_marker_yields_no_payload() {
    let stdout = format!("{ROSTER_START}\n[{{\"id\":\"main\"}}");
    assert_eq!(extract_payload(&stdout), None);
}

#[test]
fn a_missing_opening_marker_yields_no_payload() {
    let stdout = format!("[]\n{ROSTER_END}");
    assert_eq!(extract_payload(&stdout), None);
}

#[test]
fn an_unknown_harness_is_unsupported_rather_than_failed() {
    // The distinction drives the UI: unsupported offers manual entry, failure
    // offers a retry.
    let result = probe_local_harness_agents("hermes");
    assert!(!result.supported);
    assert!(!result.ok);
    assert!(result.candidates.is_empty());
    assert_eq!(result.harness_id, "hermes");
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("manually")),
        "{:?}",
        result.error
    );
}

#[test]
fn a_supported_harness_reports_supported_even_when_absent() {
    // OpenClaw need not be installed on the machine running the tests: the point
    // is that "Buzz knows how to ask" is independent of "the answer succeeded".
    let result = probe_local_harness_agents("openclaw");
    assert!(result.supported);
    assert_eq!(result.harness_id, "openclaw");
}
