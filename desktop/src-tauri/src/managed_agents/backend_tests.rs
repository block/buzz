use super::*;

#[test]
fn redact_secrets_replaces_nsec() {
    let s = "key=nsec1abc123def456 other";
    let r = redact_secrets(s);
    assert!(r.contains("[REDACTED]"));
    assert!(!r.contains("nsec1abc123def456"));
}

#[test]
fn redact_secrets_replaces_token() {
    let s = r#"{"token":"sprt_tok_xyz789"}"#;
    let r = redact_secrets(s);
    assert!(r.contains("[REDACTED]"));
    assert!(!r.contains("sprt_tok_xyz789"));
}

#[test]
fn redact_secrets_with_extras_scrubs_user_env_values() {
    // If a provider echoes back a user-supplied API key in its error
    // output, the desktop must not surface that secret unredacted via
    // `last_error`. We scrub the literal values that came from the
    // request's `agent.env_vars`.
    let secret = "sk-ant-api03-abc123def456";
    let stderr = format!("auth failed with key {secret} on host api.anthropic.com");
    let r = redact_secrets_with(&stderr, &[secret]);
    assert!(r.contains("[REDACTED]"));
    assert!(!r.contains(secret));
}

#[test]
fn redact_secrets_with_extras_skips_short_values() {
    // Don't scrub values shorter than 4 chars — too noisy.
    let r = redact_secrets_with("error code: 42", &["42"]);
    assert!(r.contains("42"));
}

/// GitHub tokens are recognised by shape, so one that never passed through
/// our environment — embedded in a remote URL an installer echoes — is
/// still scrubbed. The scan runs to the next whitespace or quote, so the
/// rest of the URL goes with it; over-redaction is the safe direction.
#[test]
fn redact_secrets_with_scrubs_github_token_prefixes() {
    for token in [
        "ghp_abcdefghij0123456789",
        "gho_abcdefghij0123456789",
        "ghu_abcdefghij0123456789",
        "ghs_abcdefghij0123456789",
        "ghr_abcdefghij0123456789",
        "github_pat_abcdefghij0123456789",
    ] {
        let r = redact_secrets_with(&format!("cloning https://{token}@github.com/o/r now"), &[]);
        assert!(!r.contains(token), "leaked {token}: {r}");
        assert!(r.contains("[REDACTED]"), "got: {r}");
        assert!(r.contains("cloning"), "scan must stop at whitespace: {r}");
        assert!(r.ends_with(" now"), "scan must stop at whitespace: {r}");
    }
}

#[test]
fn redact_secrets_with_extras_terminates_when_value_substring_of_marker() {
    // Regression: an earlier impl used `while let Some(pos) = find(value)`
    // which never terminates if the user's env value is a substring of
    // the replacement marker `[REDACTED]` — each replacement
    // reintroduces the same text. Now uses `str::replace` (single-pass).
    for value in ["REDACTED", "EDACTE", "REDA", "ACTED"] {
        let r = redact_secrets_with(&format!("leak={value}"), &[value]);
        assert!(r.contains("[REDACTED]"));
    }
}

#[test]
fn redact_secrets_with_extras_handles_overlapping_secrets() {
    // Longer entries get scrubbed first so the substring "abc12" isn't
    // matched before "abc123" is consumed.
    let s = "key1=abc123 key2=abc12";
    let r = redact_secrets_with(s, &["abc12", "abc123"]);
    assert!(!r.contains("abc123"));
    assert!(!r.contains("abc12 "));
}

#[test]
fn env_secrets_from_request_extracts_string_values() {
    let req = serde_json::json!({
        "op": "deploy",
        "agent": {
            "env_vars": {
                "ANTHROPIC_API_KEY": "sk-ant-test",
                "EMPTY": "",
                "NUMERIC": 42,
            },
        },
    });
    let secrets = env_secrets_from_request(&req);
    assert!(secrets.iter().any(|v| v == "sk-ant-test"));
    // Empty and non-string values are filtered out.
    assert_eq!(secrets.len(), 1);
}

#[test]
fn env_secrets_from_request_handles_missing_shape() {
    assert!(env_secrets_from_request(&serde_json::json!({})).is_empty());
    assert!(env_secrets_from_request(&serde_json::json!({"agent": {}})).is_empty());
    assert!(env_secrets_from_request(&serde_json::json!({"agent": {"env_vars": null}})).is_empty());
}

#[test]
fn redact_env_values_in_scrubs_map_values() {
    let mut env = std::collections::BTreeMap::new();
    env.insert("ANTHROPIC_API_KEY".to_string(), "sk-ant-real".to_string());
    env.insert("EMPTY".to_string(), String::new());
    let stderr = "auth=sk-ant-real failed; other context";
    let r = redact_env_values_in(stderr, &env);
    assert!(!r.contains("sk-ant-real"));
    assert!(r.contains("[REDACTED]"));
}

#[test]
fn env_secrets_from_request_includes_resolved_launch_maps() {
    let req = serde_json::json!({
        "agent": {
            "env_vars": {"LEGACY": "legacy-secret"},
            "launch": {
                "env": {"PERSONA": "persona-secret"},
                "policy_env": {"POLICY": "policy-secret"}
            }
        }
    });
    let secrets = env_secrets_from_request(&req);
    assert_eq!(secrets.len(), 3);
    for secret in ["legacy-secret", "persona-secret", "policy-secret"] {
        assert!(secrets.iter().any(|candidate| candidate == secret));
    }
}

#[cfg(unix)]
fn write_test_provider(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(unix)]
#[test]
fn provider_deploy_negotiates_and_deploys_the_same_staged_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let provider = directory.path().join("provider");
    let log = directory.path().join("invocations");
    let body = format!(
        r#"read request
printf '%s\n' "$0" >> '{}'
case "$request" in
  *\"op\":\"info\"*) printf '%s\n' '{{"ok":true,"name":"test","version":"1.0.0","protocol_version":1,"description":"test provider","config_schema":{{}}}}' ;;
  *\"op\":\"deploy\"*) printf '%s\n' '{{"ok":true,"agent_id":"remote-1"}}' ;;
esac"#,
        log.display()
    );
    write_test_provider(&provider, &body);

    let id = provider_deploy(&provider, &serde_json::json!({}), &serde_json::json!({}))
        .expect("staged deploy");
    assert_eq!(id, "remote-1");
    let paths: Vec<_> = std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0], paths[1]);
    assert_ne!(Path::new(&paths[0]), provider);
    assert!(
        !Path::new(&paths[0]).exists(),
        "staging directory must be deleted"
    );
}

#[cfg(unix)]
fn replacement_provider() -> &'static str {
    r#"#!/bin/sh
set -eu
read request
case "$request" in
  *\"op\":\"info\"*) printf '%s\n' '{"ok":true,"name":"replacement","version":"9.9.9","protocol_version":1,"description":"replacement provider","config_schema":{}}' ;;
  *\"op\":\"deploy\"*) printf '%s\n' '{"ok":true,"agent_id":"replacement-bytes-ran"}' ;;
esac
"#
}

#[cfg(unix)]
#[test]
fn provider_deploy_uses_staged_bytes_after_same_inode_source_rewrite() {
    use std::os::unix::fs::MetadataExt;

    let directory = tempfile::tempdir().unwrap();
    let provider = directory.path().join("provider");
    let replacement = directory.path().join("replacement");
    std::fs::write(&replacement, replacement_provider()).unwrap();
    let body = format!(
        r#"read request
case "$request" in
  *\"op\":\"info\"*)
    cat '{}' > '{}'
    chmod 700 '{}'
    printf '%s\n' '{{"ok":true,"name":"original","version":"1.0.0","protocol_version":1,"description":"original provider","config_schema":{{}}}}'
    ;;
  *\"op\":\"deploy\"*) printf '%s\n' '{{"ok":true,"agent_id":"original-staged-bytes"}}' ;;
esac"#,
        replacement.display(),
        provider.display(),
        provider.display(),
    );
    write_test_provider(&provider, &body);
    let inode_before = std::fs::metadata(&provider).unwrap().ino();

    let id = provider_deploy(&provider, &serde_json::json!({}), &serde_json::json!({}))
        .expect("deploy from immutable staged copy");

    assert_eq!(id, "original-staged-bytes");
    assert_eq!(
        std::fs::metadata(&provider).unwrap().ino(),
        inode_before,
        "test must rewrite the source binary in place"
    );
    assert_eq!(
        std::fs::read_to_string(&provider).unwrap(),
        replacement_provider(),
        "source pathname must contain replacement bytes before deploy"
    );
}

#[cfg(unix)]
#[test]
fn provider_deploy_uses_staged_bytes_after_source_pathname_replacement() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let directory = tempfile::tempdir().unwrap();
    let provider = directory.path().join("provider");
    let replacement = directory.path().join("replacement");
    std::fs::write(&replacement, replacement_provider()).unwrap();
    std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o700)).unwrap();
    let body = format!(
        r#"read request
case "$request" in
  *\"op\":\"info\"*)
    mv '{}' '{}'
    printf '%s\n' '{{"ok":true,"name":"original","version":"1.0.0","protocol_version":1,"description":"original provider","config_schema":{{}}}}'
    ;;
  *\"op\":\"deploy\"*) printf '%s\n' '{{"ok":true,"agent_id":"original-staged-bytes"}}' ;;
esac"#,
        replacement.display(),
        provider.display(),
    );
    write_test_provider(&provider, &body);
    let inode_before = std::fs::metadata(&provider).unwrap().ino();

    let id = provider_deploy(&provider, &serde_json::json!({}), &serde_json::json!({}))
        .expect("deploy from immutable staged copy");

    assert_eq!(id, "original-staged-bytes");
    assert_ne!(
        std::fs::metadata(&provider).unwrap().ino(),
        inode_before,
        "test must replace the source pathname with a different inode"
    );
    assert_eq!(
        std::fs::read_to_string(&provider).unwrap(),
        replacement_provider(),
        "source pathname must contain replacement bytes before deploy"
    );
}

#[cfg(unix)]
#[test]
fn provider_deploy_refuses_mismatch_before_sending_agent_secret() {
    let directory = tempfile::tempdir().unwrap();
    let provider = directory.path().join("provider");
    let marker = directory.path().join("deploy-received");
    let body = format!(
        r#"read request
case "$request" in
  *\"op\":\"info\"*) printf '%s\n' '{{"ok":true,"name":"test","version":"2.0.0","protocol_version":2,"description":"test provider","config_schema":{{}}}}' ;;
  *\"op\":\"deploy\"*) touch '{}'; printf '%s\n' '{{"ok":true,"agent_id":"bad"}}' ;;
esac"#,
        marker.display()
    );
    write_test_provider(&provider, &body);

    let error = provider_deploy(
        &provider,
        &serde_json::json!({"private_key_nsec": "nsec1must-not-cross"}),
        &serde_json::json!({}),
    )
    .unwrap_err();
    assert!(error.contains("protocol version 2"), "{error}");
    assert!(!marker.exists());
    assert!(!error.contains("nsec1must-not-cross"));
}

#[cfg(unix)]
#[test]
fn provider_deploy_requires_an_explicit_integer_protocol_version() {
    let directory = tempfile::tempdir().unwrap();
    let provider = directory.path().join("provider");
    write_test_provider(
        &provider,
        r#"read request
printf '%s\n' '{"ok":true,"version":"1.0.0"}'"#,
    );

    let error =
        provider_deploy(&provider, &serde_json::json!({}), &serde_json::json!({})).unwrap_err();
    assert!(
        error.contains("missing integer protocol_version"),
        "{error}"
    );
}

#[test]
fn provider_info_requires_the_complete_flat_wire_shape() {
    let complete = serde_json::json!({
        "ok": true,
        "name": "kubernetes",
        "version": "1.0.0",
        "protocol_version": 1,
        "description": "Kubernetes provider",
        "config_schema": {}
    });
    assert!(validate_provider_info(&complete).is_ok());

    let mut missing = complete.clone();
    missing.as_object_mut().unwrap().remove("config_schema");
    assert!(validate_provider_info(&missing)
        .unwrap_err()
        .contains("config_schema"));

    let mut nested = complete;
    nested.as_object_mut().unwrap().insert(
        "provider".into(),
        serde_json::json!({"protocol_version": 1}),
    );
    assert!(validate_provider_info(&nested)
        .unwrap_err()
        .contains("unknown field provider"));
}

#[test]
fn validate_provider_config_rejects_secret_key() {
    let cfg = serde_json::json!({"api_key": "val"});
    assert!(validate_provider_config(&cfg).is_err());
}

#[test]
fn validate_provider_config_rejects_nested() {
    let cfg = serde_json::json!({"region": {"us": "east"}});
    assert!(validate_provider_config(&cfg).is_err());
}

#[test]
fn validate_provider_config_accepts_scalars() {
    let cfg = serde_json::json!({"region": "us-east-1", "tier": "standard"});
    assert!(validate_provider_config(&cfg).is_ok());
}

#[test]
fn validate_provider_config_allows_key_as_substring() {
    // "keyboard", "monkey" contain "key" as substring but not as a word segment.
    let cfg = serde_json::json!({"keyboard_layout": "us", "monkey_wrench": "tight"});
    assert!(validate_provider_config(&cfg).is_ok());
}

#[test]
fn validate_provider_config_rejects_camel_case_secrets() {
    assert!(validate_provider_config(&serde_json::json!({"apiKey": "val"})).is_err());
    assert!(validate_provider_config(&serde_json::json!({"accessToken": "val"})).is_err());
    assert!(validate_provider_config(&serde_json::json!({"clientSecret": "val"})).is_err());
    // ALL-CAPS variants
    assert!(validate_provider_config(&serde_json::json!({"apiKEY": "val"})).is_err());
    assert!(validate_provider_config(&serde_json::json!({"accessTOKEN": "val"})).is_err());
}

#[test]
fn split_config_key_handles_all_styles() {
    assert_eq!(split_config_key("apiKey"), vec!["api", "key"]);
    assert_eq!(split_config_key("access_token"), vec!["access", "token"]);
    assert_eq!(split_config_key("keyboard"), vec!["keyboard"]);
    assert_eq!(split_config_key("client-secret"), vec!["client", "secret"]);
    // Acronym runs stay together
    assert_eq!(split_config_key("APIKey"), vec!["api", "key"]);
    assert_eq!(split_config_key("apiKEY"), vec!["api", "key"]);
    assert_eq!(split_config_key("accessTOKEN"), vec!["access", "token"]);
    assert_eq!(split_config_key("MyAPIKey"), vec!["my", "api", "key"]);
}

#[test]
fn provider_filename_strips_the_windows_extension() {
    assert_eq!(
        provider_id_from_filename("buzz-backend-kubernetes"),
        Some("kubernetes")
    );
    assert_eq!(
        provider_id_from_filename("buzz-backend-kubernetes.exe"),
        Some("kubernetes")
    );
    assert_eq!(
        provider_id_from_filename("buzz-backend-kubernetes.EXE"),
        Some("kubernetes")
    );
    assert_eq!(
        provider_id_from_filename("buzz-backend-kubernetes.bat"),
        Some("kubernetes")
    );
    assert_eq!(
        provider_id_from_filename("buzz-backend-kubernetes.CMD"),
        Some("kubernetes")
    );
    assert_eq!(
        provider_id_from_filename("buzz-backend-my-provider"),
        Some("my-provider")
    );
    assert_eq!(provider_id_from_filename("other"), None);
}

#[test]
fn resolve_provider_binary_rejects_invalid_ids() {
    // Path traversal
    assert!(resolve_provider_binary("../evil").is_err());
    // Empty
    assert!(resolve_provider_binary("").is_err());
    // Uppercase
    assert!(resolve_provider_binary("MyProvider").is_err());
    // Spaces
    assert!(resolve_provider_binary("my provider").is_err());
    // Shell metacharacters
    assert!(resolve_provider_binary("foo;rm -rf /").is_err());
    // Valid format but not on PATH — should fail with "not found"
    assert!(resolve_provider_binary("nonexistent-test-id-12345").is_err());
}

#[test]
fn resolve_provider_binary_accepts_valid_id_format() {
    // Valid ID format should pass validation. If the binary happens to
    // exist on PATH, Ok is returned; otherwise Err contains "not found"
    // (not "invalid provider ID"). Either outcome proves validation passed.
    match resolve_provider_binary("zzz-nonexistent-test-provider") {
        Ok(_) => {} // unlikely but fine — binary exists
        Err(e) => assert!(
            e.contains("not found"),
            "expected 'not found' error, got: {e}"
        ),
    }
}
