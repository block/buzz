use std::collections::BTreeMap;

use super::{
    display_invalid_key, filter_derived_provider_model_env_vars, is_derived_provider_model_key,
    is_reserved_env_key, is_well_formed_env_key, merged_user_env, validate_user_env_keys,
    DERIVED_PROVIDER_MODEL_ENV_KEYS, MAX_ENV_TOTAL_BYTES, MAX_ENV_VALUE_BYTES, RESERVED_ENV_KEYS,
};

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

// ── merged_user_env: layering ──────────────────────────────────────

#[test]
fn merged_env_empty_inputs_returns_empty() {
    let merged = merged_user_env(&BTreeMap::new(), &BTreeMap::new());
    assert!(merged.is_empty());
}

#[test]
fn merged_env_persona_only_is_returned_verbatim() {
    let persona = map(&[("ANTHROPIC_API_KEY", "p-key"), ("FOO", "1")]);
    let merged = merged_user_env(&persona, &BTreeMap::new());
    assert_eq!(
        merged.get("ANTHROPIC_API_KEY").map(String::as_str),
        Some("p-key")
    );
    assert_eq!(merged.get("FOO").map(String::as_str), Some("1"));
    assert_eq!(merged.len(), 2);
}

#[test]
fn merged_env_agent_only_is_returned_verbatim() {
    let agent = map(&[("BAR", "a-val")]);
    let merged = merged_user_env(&BTreeMap::new(), &agent);
    assert_eq!(merged.get("BAR").map(String::as_str), Some("a-val"));
    assert_eq!(merged.len(), 1);
}

#[test]
fn merged_env_agent_overrides_persona_on_collision() {
    // Per Tyler's rule: "If I put it in overrides, I want it to override."
    let persona = map(&[("ANTHROPIC_API_KEY", "from-persona"), ("MODEL", "claude")]);
    let agent = map(&[("ANTHROPIC_API_KEY", "from-agent")]);
    let merged = merged_user_env(&persona, &agent);
    assert_eq!(
        merged.get("ANTHROPIC_API_KEY").map(String::as_str),
        Some("from-agent")
    );
    assert_eq!(merged.get("MODEL").map(String::as_str), Some("claude"));
    assert_eq!(merged.len(), 2);
}

#[test]
fn merged_env_agent_can_add_new_keys() {
    let persona = map(&[("A", "1")]);
    let agent = map(&[("B", "2")]);
    let merged = merged_user_env(&persona, &agent);
    assert_eq!(merged.get("A").map(String::as_str), Some("1"));
    assert_eq!(merged.get("B").map(String::as_str), Some("2"));
    assert_eq!(merged.len(), 2);
}

#[test]
fn merged_env_empty_value_is_passed_through() {
    // No special-casing: empty string is a valid env value.
    let agent = map(&[("CLEARED_TO_EMPTY", "")]);
    let merged = merged_user_env(&BTreeMap::new(), &agent);
    assert_eq!(merged.get("CLEARED_TO_EMPTY").map(String::as_str), Some(""));
}

#[test]
fn merged_env_does_not_mutate_inputs() {
    let persona = map(&[("A", "1")]);
    let agent = map(&[("A", "2")]);
    let _ = merged_user_env(&persona, &agent);
    assert_eq!(persona.get("A").map(String::as_str), Some("1"));
    assert_eq!(agent.get("A").map(String::as_str), Some("2"));
}

// ── reserved-key filter ────────────────────────────────────────────

#[test]
fn merged_env_strips_reserved_keys_from_persona() {
    // Defense-in-depth: even if a reserved key sneaks into on-disk
    // persona data (e.g. older record from before validation existed),
    // it must be stripped before reaching the child process.
    let persona = map(&[
        ("BUZZ_PRIVATE_KEY", "nsec1evil"),
        ("ANTHROPIC_API_KEY", "ok"),
    ]);
    let merged = merged_user_env(&persona, &BTreeMap::new());
    assert!(!merged.contains_key("BUZZ_PRIVATE_KEY"));
    assert_eq!(
        merged.get("ANTHROPIC_API_KEY").map(String::as_str),
        Some("ok")
    );
}

#[test]
fn merged_env_strips_reserved_keys_from_agent() {
    let agent = map(&[
        ("NOSTR_PRIVATE_KEY", "nsec1evil"),
        ("BUZZ_AUTH_TAG", "{}"),
        ("FOO", "1"),
    ]);
    let merged = merged_user_env(&BTreeMap::new(), &agent);
    assert!(!merged.contains_key("NOSTR_PRIVATE_KEY"));
    assert!(!merged.contains_key("BUZZ_AUTH_TAG"));
    assert_eq!(merged.get("FOO").map(String::as_str), Some("1"));
    assert_eq!(merged.len(), 1);
}

#[test]
fn merged_env_strips_reserved_case_insensitive() {
    // Unix env vars are case-sensitive at the syscall level, but we
    // refuse close-typo variants too — a lowercase `buzz_private_key`
    // is almost certainly a footgun, not a legitimate use.
    let agent = map(&[("buzz_private_key", "x"), ("Buzz_Auth_Tag", "y")]);
    let merged = merged_user_env(&BTreeMap::new(), &agent);
    assert!(merged.is_empty());
}

#[test]
fn is_reserved_recognises_full_list() {
    for key in RESERVED_ENV_KEYS {
        assert!(is_reserved_env_key(key), "{key} should be reserved");
    }
    assert!(!is_reserved_env_key("GOOSE_MODE"));
    assert!(!is_reserved_env_key("ANTHROPIC_API_KEY"));
    assert!(!is_reserved_env_key("BUZZ_ACP_MODEL")); // behavior knob
}

#[test]
fn reserved_keys_include_agent_owner_for_legacy_records() {
    // Legacy records without auth_tag fall back to BUZZ_ACP_AGENT_OWNER
    // to enforce the respond-to gate. Must not be user-overridable.
    assert!(is_reserved_env_key("BUZZ_ACP_AGENT_OWNER"));
    let agent = map(&[("BUZZ_ACP_AGENT_OWNER", "imposter")]);
    let merged = merged_user_env(&BTreeMap::new(), &agent);
    assert!(merged.is_empty());
}

#[test]
fn reserved_keys_include_respond_to_gate() {
    // Respond-to mode + allowlist control who the agent answers.
    // Overriding via env_vars would let the running agent answer
    // anyone even when the UI/record says owner-only.
    for key in ["BUZZ_ACP_RESPOND_TO", "BUZZ_ACP_RESPOND_TO_ALLOWLIST"] {
        assert!(is_reserved_env_key(key), "{key} should be reserved");
        let agent = map(&[(key, "anyone")]);
        let merged = merged_user_env(&BTreeMap::new(), &agent);
        assert!(merged.is_empty(), "{key} should be stripped");
    }
}

#[test]
fn reserved_keys_include_code_execution_surface() {
    // The agent/MCP command + args are what Buzz actually exec's.
    // Overriding lets the user run arbitrary code as the agent.
    for key in [
        "BUZZ_ACP_AGENT_COMMAND",
        "BUZZ_ACP_AGENT_ARGS",
        "BUZZ_ACP_MCP_COMMAND",
    ] {
        assert!(is_reserved_env_key(key), "{key} should be reserved");
    }
}

#[test]
fn reserved_keys_include_relay_url() {
    // Overriding the relay URL could redirect the agent to an
    // attacker-controlled relay.
    assert!(is_reserved_env_key("BUZZ_RELAY_URL"));
    let agent = map(&[("BUZZ_RELAY_URL", "ws://attacker.example")]);
    let merged = merged_user_env(&BTreeMap::new(), &agent);
    assert!(merged.is_empty());
}

// ── validate_user_env_keys ─────────────────────────────────────────

#[test]
fn validate_keys_accepts_normal_env() {
    let env = map(&[("ANTHROPIC_API_KEY", "k"), ("GOOSE_PROVIDER", "anthropic")]);
    assert!(validate_user_env_keys(&env).is_ok());
}

#[test]
fn validate_keys_rejects_reserved() {
    let env = map(&[("BUZZ_PRIVATE_KEY", "nsec1evil")]);
    let err = validate_user_env_keys(&env).unwrap_err();
    assert!(err.contains("BUZZ_PRIVATE_KEY"), "got: {err}");
    assert!(err.contains("reserved"), "got: {err}");
}

#[test]
fn validate_keys_lists_all_reserved_keys_found() {
    let env = map(&[
        ("BUZZ_PRIVATE_KEY", "x"),
        ("NOSTR_PRIVATE_KEY", "y"),
        ("ANTHROPIC_API_KEY", "ok"),
    ]);
    let err = validate_user_env_keys(&env).unwrap_err();
    assert!(err.contains("BUZZ_PRIVATE_KEY"));
    assert!(err.contains("NOSTR_PRIVATE_KEY"));
}

#[test]
fn validate_keys_rejects_empty_key() {
    let env = map(&[("", "value")]);
    let err = validate_user_env_keys(&env).unwrap_err();
    assert!(err.contains("(empty)"), "got: {err}");
    assert!(err.contains("[A-Za-z_]"), "got: {err}");
}

#[test]
fn validate_keys_accepts_empty_map() {
    assert!(validate_user_env_keys(&BTreeMap::new()).is_ok());
}

// ── malformed-key rejection (=-in-key bypass and friends) ──────────
//
// Rust's `Command::env(k, v)` will accept a key containing `=` and
// pass it straight into the child's environ block, where
// `getenv("PREFIX")` matches anything after the first `=`. Concretely:
// `c.env("BUZZ_AUTH_TAG=x", "forged")` results in the child seeing
// `BUZZ_AUTH_TAG=x=forged` and `getenv("BUZZ_AUTH_TAG") == "x=forged"`.
// That bypasses our reserved-key check, which compares strings.
// These tests pin the fix at the validator boundary.

#[test]
fn is_well_formed_accepts_posix_keys() {
    for key in [
        "FOO",
        "FOO_BAR",
        "_LEADING_UNDERSCORE",
        "MIXED_Case_Letters",
        "WITH_DIGITS_123",
        "A", // single char
    ] {
        assert!(is_well_formed_env_key(key), "{key} should be well-formed");
    }
}

#[test]
fn is_well_formed_rejects_malformed_keys() {
    for key in [
        "",                  // empty
        "=",                 // bare equals
        "BUZZ_AUTH_TAG=x",   // =-in-key bypass
        "BUZZ_PRIVATE_KEY=", // trailing equals
        "FOO BAR",           // space
        " FOO",              // leading whitespace
        "FOO\nBAR",          // newline
        "FOO\0BAR",          // NUL
        "123_LEADING_DIGIT", // POSIX forbids leading digit
        "FOO-BAR",           // hyphen
        "FOO.BAR",           // dot
        "FOO/BAR",           // slash
        "ünicode_key",       // non-ASCII
    ] {
        assert!(!is_well_formed_env_key(key), "{key:?} should be malformed");
    }
}

#[test]
fn validate_keys_rejects_equals_in_key_bypass() {
    // The actual exploit: `BUZZ_AUTH_TAG=x` smuggles a value past
    // the reserved-key string compare and into the child's environ.
    let env = map(&[("BUZZ_AUTH_TAG=x", "forged")]);
    let err = validate_user_env_keys(&env).unwrap_err();
    assert!(err.contains("[A-Za-z_]"), "got: {err}");
    // After P2 fix the key is truncated at `=` in the error to avoid
    // surfacing pasted secrets — only the prefix should appear, with an
    // ellipsis marking that we elided trailing content.
    assert!(err.contains("BUZZ_AUTH_TAG"), "got: {err}");
    assert!(err.contains('…'), "expected ellipsis marker: {err}");
    assert!(!err.contains("=x"), "leak of value past `=`: {err}");
}

#[test]
fn validate_keys_rejects_whitespace_and_nul() {
    let env = map(&[("FOO BAR", "v"), ("HAS\0NUL", "v")]);
    let err = validate_user_env_keys(&env).unwrap_err();
    assert!(err.contains("[A-Za-z_]"), "got: {err}");
}

#[test]
fn validate_keys_reports_malformed_before_reserved() {
    // If a key is malformed it's not worth telling the user "and by
    // the way that other key is reserved" — they've got a typo to fix
    // first. Ordering is a UX detail but pinning it stops the message
    // from churning.
    let env = map(&[("BUZZ_AUTH_TAG=x", "v"), ("BUZZ_PRIVATE_KEY", "v")]);
    let err = validate_user_env_keys(&env).unwrap_err();
    assert!(err.contains("[A-Za-z_]"), "got: {err}");
    assert!(!err.contains("reserved"), "got: {err}");
}

#[test]
fn merged_env_drops_malformed_keys() {
    // Defense in depth: on-disk records written before the validator
    // tightened must not be able to smuggle reserved keys through.
    let agent = map(&[
        ("BUZZ_AUTH_TAG=x", "forged"),
        ("FOO=bar", "v"),
        ("LEGIT", "ok"),
    ]);
    let merged = merged_user_env(&BTreeMap::new(), &agent);
    assert!(!merged.contains_key("BUZZ_AUTH_TAG=x"));
    assert!(!merged.contains_key("FOO=bar"));
    assert_eq!(merged.get("LEGIT").map(String::as_str), Some("ok"));
    assert_eq!(merged.len(), 1);
}

#[test]
fn display_invalid_key_truncates_at_equals_to_hide_secrets() {
    // The exact bug class: a user pastes `KEY=sk-secret` into the key
    // field. The validator must reject it AND must not echo the secret
    // back in the error/log.
    let rendered = display_invalid_key("ANTHROPIC_API_KEY=sk-ant-XXXXXXXXXXXX");
    assert!(!rendered.contains("sk-"), "rendered: {rendered}");
    assert!(
        rendered.starts_with("ANTHROPIC_API_KEY"),
        "rendered: {rendered}"
    );
}

#[test]
fn display_invalid_key_replaces_control_bytes() {
    // NUL or newline in the key field would otherwise corrupt log lines.
    let rendered = display_invalid_key("FOO\nBAR\0BAZ");
    assert!(!rendered.contains('\n'), "rendered: {rendered}");
    assert!(!rendered.contains('\0'), "rendered: {rendered}");
}

#[test]
fn display_invalid_key_caps_long_keys() {
    let key = "A".repeat(200);
    let rendered = display_invalid_key(&key);
    // 64-char cap + ellipsis marker.
    assert!(
        rendered.chars().count() <= 65,
        "rendered len: {}",
        rendered.chars().count()
    );
    assert!(rendered.ends_with('…'), "rendered: {rendered}");
}

#[test]
fn validate_user_env_keys_error_does_not_leak_value_after_equals() {
    let env = map(&[("ANTHROPIC_API_KEY=sk-ant-XXXX-leak", "ignored")]);
    let err = validate_user_env_keys(&env).unwrap_err();
    assert!(!err.contains("sk-ant"), "err leaked secret: {err}");
}

#[test]
fn validate_rejects_nul_byte_in_value() {
    let env = map(&[("FOO", "ok\0bad")]);
    let err = validate_user_env_keys(&env).unwrap_err();
    assert!(err.contains("NUL"), "got: {err}");
    assert!(err.contains("FOO"), "got: {err}");
    // Generic message — must not echo any portion of the value.
    assert!(!err.contains("ok"), "leak: {err}");
    assert!(!err.contains("bad"), "leak: {err}");
}

#[test]
fn validate_rejects_oversize_value() {
    let big = "x".repeat(MAX_ENV_VALUE_BYTES + 1);
    let env = map(&[("FOO", big.as_str())]);
    let err = validate_user_env_keys(&env).unwrap_err();
    assert!(err.contains("per-value limit"), "got: {err}");
    assert!(err.contains("FOO"), "got: {err}");
    // Don't echo the value.
    assert!(!err.contains("xxx"), "leak: {err}");
}

#[test]
fn validate_rejects_oversize_total_payload() {
    // Many medium values that each pass per-value but sum past the total.
    let val = "y".repeat(MAX_ENV_VALUE_BYTES);
    let entries: Vec<(String, String)> = (0..((MAX_ENV_TOTAL_BYTES / MAX_ENV_VALUE_BYTES) + 1))
        .map(|i| (format!("K{i}"), val.clone()))
        .collect();
    let env: BTreeMap<String, String> = entries.into_iter().collect();
    let err = validate_user_env_keys(&env).unwrap_err();
    assert!(err.contains("total env var payload"), "got: {err}");
}

#[test]
fn merged_env_drops_value_with_nul_byte() {
    // Defense in depth — older on-disk record. `Command::env` would panic
    // on a NUL in a value; the runtime filter must strip it.
    let agent = map(&[("FOO", "ok\0bad"), ("LEGIT", "v")]);
    let merged = merged_user_env(&BTreeMap::new(), &agent);
    assert!(!merged.contains_key("FOO"));
    assert_eq!(merged.get("LEGIT").map(String::as_str), Some("v"));
}

#[test]
fn merged_env_drops_oversize_value() {
    let big = "z".repeat(MAX_ENV_VALUE_BYTES + 1);
    let agent: BTreeMap<String, String> = [
        ("HUGE".to_string(), big),
        ("LEGIT".to_string(), "v".to_string()),
    ]
    .into_iter()
    .collect();
    let merged = merged_user_env(&BTreeMap::new(), &agent);
    assert!(!merged.contains_key("HUGE"));
    assert_eq!(merged.get("LEGIT").map(String::as_str), Some("v"));
}

// ── derived provider/model key filter ──────────────────────────────
//
// Pack import must strip derived env keys (GOOSE_MODEL, GOOSE_PROVIDER,
// BUZZ_AGENT_MODEL, BUZZ_AGENT_PROVIDER) so they don't shadow the
// structured AgentDefinition.model / AgentDefinition.provider fields after
// the user edits them in the UI.

#[test]
fn is_derived_key_matches_all_known_keys() {
    for key in DERIVED_PROVIDER_MODEL_ENV_KEYS {
        assert!(
            is_derived_provider_model_key(key),
            "expected `{key}` to be recognized as derived"
        );
    }
}

#[test]
fn is_derived_key_is_case_insensitive() {
    assert!(is_derived_provider_model_key("goose_model"));
    assert!(is_derived_provider_model_key("Goose_Provider"));
    assert!(is_derived_provider_model_key("buzz_agent_model"));
    assert!(is_derived_provider_model_key("BUZZ_AGENT_PROVIDER"));
}

#[test]
fn is_derived_key_does_not_match_unrelated_keys() {
    assert!(!is_derived_provider_model_key("GOOSE_TEMPERATURE"));
    assert!(!is_derived_provider_model_key("GOOSE_CONTEXT_LIMIT"));
    assert!(!is_derived_provider_model_key("ANTHROPIC_API_KEY"));
    assert!(!is_derived_provider_model_key("BUZZ_PRIVATE_KEY"));
    assert!(!is_derived_provider_model_key("MODEL"));
    assert!(!is_derived_provider_model_key("PROVIDER"));
}

#[test]
fn filter_derived_strips_provider_model_keys_preserves_rest() {
    let input = vec![
        ("GOOSE_MODEL".to_string(), "old-model".to_string()),
        ("GOOSE_PROVIDER".to_string(), "old-provider".to_string()),
        ("BUZZ_AGENT_MODEL".to_string(), "gpt-4o".to_string()),
        ("BUZZ_AGENT_PROVIDER".to_string(), "openai".to_string()),
        ("GOOSE_TEMPERATURE".to_string(), "0.7".to_string()),
        ("GOOSE_CONTEXT_LIMIT".to_string(), "128000".to_string()),
        ("CUSTOM_KEY".to_string(), "custom-value".to_string()),
    ];

    let filtered = filter_derived_provider_model_env_vars(input);

    // Derived keys must be gone.
    assert!(!filtered.contains_key("GOOSE_MODEL"));
    assert!(!filtered.contains_key("GOOSE_PROVIDER"));
    assert!(!filtered.contains_key("BUZZ_AGENT_MODEL"));
    assert!(!filtered.contains_key("BUZZ_AGENT_PROVIDER"));

    // Non-derived keys must survive.
    assert_eq!(
        filtered.get("GOOSE_TEMPERATURE").map(String::as_str),
        Some("0.7")
    );
    assert_eq!(
        filtered.get("GOOSE_CONTEXT_LIMIT").map(String::as_str),
        Some("128000")
    );
    assert_eq!(
        filtered.get("CUSTOM_KEY").map(String::as_str),
        Some("custom-value")
    );
}

#[test]
fn filter_derived_empty_input_returns_empty() {
    let filtered = filter_derived_provider_model_env_vars(std::iter::empty());
    assert!(filtered.is_empty());
}

#[test]
fn stale_derived_env_does_not_override_structured_fields() {
    // Scenario: A persona was imported WITH stale derived keys (pre-fix).
    // At merge time, `merged_user_env` passes them through (it doesn't filter).
    // The fix is at *import* time — this test documents that merged_user_env
    // is transparent, and the import filter is the correct defense.
    let stale_persona_env = map(&[
        ("BUZZ_AGENT_MODEL", "stale-model"),
        ("BUZZ_AGENT_PROVIDER", "stale-provider"),
        ("GOOSE_TEMPERATURE", "0.5"),
    ]);
    let agent_env = BTreeMap::new();

    let merged = merged_user_env(&stale_persona_env, &agent_env);

    // merged_user_env is transparent — stale keys pass through.
    assert_eq!(
        merged.get("BUZZ_AGENT_MODEL").map(String::as_str),
        Some("stale-model")
    );
    assert_eq!(
        merged.get("BUZZ_AGENT_PROVIDER").map(String::as_str),
        Some("stale-provider")
    );

    // But the import filter WOULD have caught them:
    let would_be_filtered = filter_derived_provider_model_env_vars(stale_persona_env);
    assert!(!would_be_filtered.contains_key("BUZZ_AGENT_MODEL"));
    assert!(!would_be_filtered.contains_key("BUZZ_AGENT_PROVIDER"));
    // Non-derived keys survive the filter.
    assert_eq!(
        would_be_filtered
            .get("GOOSE_TEMPERATURE")
            .map(String::as_str),
        Some("0.5")
    );
}

// ── deploy payload model precedence ────────────────────────────────

/// Documents the model precedence rule used by `build_deploy_payload`:
/// persona structured model is authoritative when present; the agent
/// record's `model` field is only a fallback.
///
/// This mirrors local spawn behavior where `runtime_metadata_env_vars`
/// derives GOOSE_MODEL from the persona's structured field, not the
/// agent record.
#[test]
fn deploy_model_precedence_persona_wins_over_record() {
    // Simulates the precedence logic from build_deploy_payload:
    //   let model = persona.model.clone().or(record.model.clone());
    let persona_model: Option<String> = Some("claude-sonnet-4-20250514".to_string());
    let record_model: Option<String> = Some("stale-record-model".to_string());

    let effective = persona_model.clone().or(record_model.clone());
    assert_eq!(effective.as_deref(), Some("claude-sonnet-4-20250514"));
}

#[test]
fn deploy_model_precedence_falls_back_to_record_when_persona_has_none() {
    let persona_model: Option<String> = None;
    let record_model: Option<String> = Some("record-model".to_string());

    let effective = persona_model.clone().or(record_model.clone());
    assert_eq!(effective.as_deref(), Some("record-model"));
}

#[test]
fn deploy_model_precedence_none_when_both_absent() {
    let persona_model: Option<String> = None;
    let record_model: Option<String> = None;

    let effective = persona_model.clone().or(record_model.clone());
    assert_eq!(effective, None);
}
