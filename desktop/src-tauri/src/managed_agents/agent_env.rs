//! Build-time agent env passthrough.
//!
//! Internal builds (buzz-releases) bake arbitrary `KEY=VALUE` pairs into the
//! binary via `BUZZ_BUILD_AGENT_ENV` (base64-encoded, newline-delimited).
//! OSS builds leave the compile-time var unset — nothing is injected.

use std::collections::BTreeMap;

use base64::Engine as _;

/// Return the baked-in build-time env pairs as a map.
///
/// Internal builds (buzz-releases) bake provider/model defaults and arbitrary
/// `KEY=VALUE` pairs into the binary at compile time. This function returns
/// those pairs as an owned map so callers can fold them into an in-process env
/// at the **lowest** precedence layer — user/persona values layered on top
/// override these baked defaults (last-write-wins, matching the existing
/// subprocess ordering where user env is written last).
///
/// OSS builds leave all `option_env!` vars unset, so this returns an empty
/// map — a safe no-op.
pub(crate) fn baked_build_env() -> BTreeMap<String, String> {
    build_env_map(
        option_env!("BUZZ_DESKTOP_BUILD_BUZZ_AGENT_PROVIDER"),
        option_env!("BUZZ_DESKTOP_BUILD_BUZZ_AGENT_MODEL"),
        option_env!("BUZZ_DESKTOP_BUILD_AGENT_ENV"),
    )
}

/// Assemble a build-time env map from optional raw bake-in values.
///
/// Separated from `baked_build_env` so the assembly logic can be exercised in
/// unit tests without relying on compile-time `option_env!` values being set.
fn build_env_map(
    raw_provider: Option<&str>,
    raw_model: Option<&str>,
    raw_agent_env: Option<&str>,
) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    if let Some(provider) = raw_provider {
        if !provider.is_empty() {
            map.insert("BUZZ_AGENT_PROVIDER".to_string(), provider.to_string());
        }
    }
    if let Some(model) = raw_model {
        if !model.is_empty() {
            map.insert("BUZZ_AGENT_MODEL".to_string(), model.to_string());
        }
    }
    if let Some(raw) = raw_agent_env {
        // The value was base64-encoded at build time so the single-line Cargo
        // output carries all KEY=VALUE pairs without truncation.
        if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(raw.as_bytes()) {
            if let Ok(text) = std::str::from_utf8(&decoded) {
                for (key, value) in parse_agent_env_lines(text) {
                    map.insert(key.to_string(), value.to_string());
                }
            }
        }
    }
    map
}

/// Fold the baked build env under `merged_env` so user/persona values win.
///
/// Returns a new map with baked pairs as the floor and `merged_env` on top.
/// OSS builds return `merged_env` unchanged (empty baked map → no-op).
pub(crate) fn discovery_env_with_baked_floor(
    merged_env: std::collections::BTreeMap<String, String>,
) -> std::collections::BTreeMap<String, String> {
    let mut env = baked_build_env();
    env.extend(merged_env);
    env
}

/// Inject baked-in provider/model defaults and generic env pairs onto `cmd`.
///
/// Call this BEFORE writing record/persona metadata env vars so that the
/// record's explicit choices (written after) override the baked defaults.
/// User-supplied `record.env_vars` (written last) always win.
pub(crate) fn build_buzz_agent_provider_defaults(cmd: &mut std::process::Command) {
    for (key, value) in baked_build_env() {
        cmd.env(key, value);
    }
}

/// Parse newline-delimited `KEY=VALUE` lines from a baked env blob.
/// Blank lines are skipped. Each non-blank line must contain `=`; the key
/// is everything before the first `=`, the value is everything after (values
/// may themselves contain `=`). Lines with an empty key are skipped.
pub(crate) fn parse_agent_env_lines(raw: &str) -> Vec<(&str, &str)> {
    raw.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let eq = line.find('=')?;
            let key = &line[..eq];
            if key.is_empty() {
                return None;
            }
            Some((key, &line[eq + 1..]))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        baked_build_env, build_buzz_agent_provider_defaults, build_env_map,
        discovery_env_with_baked_floor, parse_agent_env_lines,
    };

    #[test]
    fn buzz_agent_provider_defaults_empty_in_oss_build() {
        // OSS (and normal test) builds set neither BUZZ_BUILD_BUZZ_AGENT_*,
        // so nothing is baked in and no BUZZ_AGENT_* is injected on spawn.
        let mut cmd = std::process::Command::new("env");
        cmd.env_clear();
        build_buzz_agent_provider_defaults(&mut cmd);
        let output = cmd.output().expect("env should run");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            !stdout.contains("BUZZ_AGENT_PROVIDER="),
            "BUZZ_AGENT_PROVIDER should not be injected in OSS builds"
        );
        assert!(
            !stdout.contains("BUZZ_AGENT_MODEL="),
            "BUZZ_AGENT_MODEL should not be injected in OSS builds"
        );
        assert!(
            !stdout.contains("DATABRICKS_HOST="),
            "DATABRICKS_HOST should not be injected in OSS builds"
        );
    }

    #[test]
    fn parse_agent_env_lines_splits_on_first_equals() {
        // Value may itself contain `=` — only the first `=` is the separator.
        let pairs = parse_agent_env_lines("DATABRICKS_HOST=https://host.example.com/path?a=1");
        assert_eq!(
            pairs,
            vec![("DATABRICKS_HOST", "https://host.example.com/path?a=1")]
        );
    }

    #[test]
    fn parse_agent_env_lines_multiple_pairs() {
        let raw = "KEY_A=value_a\nKEY_B=value_b";
        let pairs = parse_agent_env_lines(raw);
        assert_eq!(pairs, vec![("KEY_A", "value_a"), ("KEY_B", "value_b")]);
    }

    #[test]
    fn parse_agent_env_lines_skips_blank_lines() {
        let raw = "KEY_A=val_a\n\n   \nKEY_B=val_b";
        let pairs = parse_agent_env_lines(raw);
        assert_eq!(pairs, vec![("KEY_A", "val_a"), ("KEY_B", "val_b")]);
    }

    #[test]
    fn parse_agent_env_lines_skips_line_without_equals() {
        // A malformed line (no `=`) is silently skipped — build.rs validates at
        // compile time; runtime parsing is defensive.
        let raw = "NO_EQUALS_HERE\nGOOD=value";
        let pairs = parse_agent_env_lines(raw);
        assert_eq!(pairs, vec![("GOOD", "value")]);
    }

    #[test]
    fn parse_agent_env_lines_skips_empty_key() {
        // `=value` has an empty key — skip it.
        let raw = "=orphan_value\nGOOD=value";
        let pairs = parse_agent_env_lines(raw);
        assert_eq!(pairs, vec![("GOOD", "value")]);
    }

    #[test]
    fn parse_agent_env_lines_empty_value_is_allowed() {
        // `KEY=` is valid — empty value is intentional (clears an env var).
        let pairs = parse_agent_env_lines("EMPTY=");
        assert_eq!(pairs, vec![("EMPTY", "")]);
    }

    #[test]
    fn parse_agent_env_lines_empty_input_returns_empty() {
        assert!(parse_agent_env_lines("").is_empty());
        assert!(parse_agent_env_lines("   \n  \n").is_empty());
    }

    // ── base64 round-trip regression ─────────────────────────────────────
    //
    // Cargo build-script output is line-oriented: a raw multiline value emitted
    // via `cargo:rustc-env=KEY=...` would be truncated to the first line.
    // build.rs base64-encodes the validated value; runtime.rs decodes it.
    // This test verifies that a 2-pair value with a URL containing `=` survives
    // the encode→decode→parse round-trip and both pairs land correctly.
    #[test]
    fn parse_agent_env_lines_base64_round_trip_preserves_all_pairs() {
        use base64::Engine as _;
        let raw =
            "DATABRICKS_HOST=https://host.example.com/path?a=1&b=2\nDATABRICKS_MODEL=some-model";
        let encoded = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
        let decoded_bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .expect("decode should succeed");
        let decoded = std::str::from_utf8(&decoded_bytes).expect("utf8 should be valid");
        let pairs = parse_agent_env_lines(decoded);
        assert_eq!(pairs.len(), 2, "both pairs must survive the round-trip");
        assert_eq!(
            pairs[0],
            ("DATABRICKS_HOST", "https://host.example.com/path?a=1&b=2")
        );
        assert_eq!(pairs[1], ("DATABRICKS_MODEL", "some-model"));
    }

    // ── baked defaults ordering regression ───────────────────────────────
    //
    // `build_buzz_agent_provider_defaults` must run BEFORE
    // `runtime_metadata_env_vars` writes the record's provider/model so that
    // record values win (last-write-wins). This test simulates the ordering by
    // writing the baked default first, then overwriting with the record value.
    #[test]
    fn baked_defaults_do_not_override_record_provider_written_after() {
        let mut cmd = std::process::Command::new("env");
        cmd.env_clear();
        // Simulate what an internal build's baked defaults would inject.
        cmd.env("BUZZ_AGENT_PROVIDER", "databricks");
        // Simulate what runtime_metadata_env_vars writes from the record (comes after).
        cmd.env("BUZZ_AGENT_PROVIDER", "anthropic");
        let output = cmd.output().expect("env should run");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("BUZZ_AGENT_PROVIDER=anthropic"),
            "record provider must win over baked default (last-write-wins)"
        );
        assert!(
            !stdout.contains("BUZZ_AGENT_PROVIDER=databricks"),
            "baked default must not survive when record provider is written after"
        );
    }

    // ── baked_build_env / build_env_map tests ─────────────────────────────

    #[test]
    fn baked_build_env_is_empty_in_oss_build() {
        // In OSS/test builds none of the BUZZ_DESKTOP_BUILD_* compile-time vars
        // are set, so baked_build_env() must return an empty map — no
        // accidental injection onto in-process discovery.
        assert!(
            baked_build_env().is_empty(),
            "baked_build_env must be empty in OSS builds (no option_env! vars set)"
        );
    }

    #[test]
    fn build_env_map_none_inputs_returns_empty() {
        assert!(build_env_map(None, None, None).is_empty());
    }

    #[test]
    fn build_env_map_provider_and_model_are_mapped() {
        let map = build_env_map(Some("databricks"), Some("my-model"), None);
        assert_eq!(
            map.get("BUZZ_AGENT_PROVIDER").map(String::as_str),
            Some("databricks")
        );
        assert_eq!(
            map.get("BUZZ_AGENT_MODEL").map(String::as_str),
            Some("my-model")
        );
    }

    #[test]
    fn build_env_map_empty_provider_is_skipped() {
        let map = build_env_map(Some(""), Some("my-model"), None);
        assert!(
            !map.contains_key("BUZZ_AGENT_PROVIDER"),
            "empty provider must be skipped"
        );
        assert_eq!(
            map.get("BUZZ_AGENT_MODEL").map(String::as_str),
            Some("my-model")
        );
    }

    #[test]
    fn build_env_map_agent_env_blob_is_decoded_and_folded() {
        use base64::Engine as _;
        let raw = "DATABRICKS_HOST=https://block-lakehouse-production.cloud.databricks.com/\nDATABRICKS_MODEL=goose-claude-opus-4-8";
        let blob = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
        let map = build_env_map(None, None, Some(&blob));
        assert_eq!(
            map.get("DATABRICKS_HOST").map(String::as_str),
            Some("https://block-lakehouse-production.cloud.databricks.com/")
        );
        assert_eq!(
            map.get("DATABRICKS_MODEL").map(String::as_str),
            Some("goose-claude-opus-4-8")
        );
    }

    #[test]
    fn build_env_map_user_env_overrides_baked_via_btreemap_extend() {
        // Validates the precedence fold used in agent_models.rs:
        //   let mut discovery_env = baked_build_env();   // floor
        //   discovery_env.extend(merged_env);            // user wins
        use base64::Engine as _;
        let raw = "DATABRICKS_HOST=https://baked.example.com/";
        let blob = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
        let mut discovery_env = build_env_map(None, None, Some(&blob));

        // User-supplied env_vars override the baked value.
        let mut user_env = std::collections::BTreeMap::new();
        user_env.insert(
            "DATABRICKS_HOST".to_string(),
            "https://user.example.com/".to_string(),
        );
        discovery_env.extend(user_env);

        assert_eq!(
            discovery_env.get("DATABRICKS_HOST").map(String::as_str),
            Some("https://user.example.com/"),
            "user env_vars must override baked DATABRICKS_HOST"
        );
    }

    #[test]
    fn discovery_env_with_baked_floor_user_key_wins_over_baked() {
        use base64::Engine as _;
        let raw = "DATABRICKS_HOST=https://baked.example.com/";
        let blob = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
        // Simulate what an internal build would produce via build_env_map.
        let _baked = build_env_map(None, None, Some(&blob));

        // In OSS test builds baked_build_env() returns empty, so we exercise
        // the helper's merge logic directly via merged_env carrying the key.
        let mut merged = std::collections::BTreeMap::new();
        merged.insert(
            "DATABRICKS_HOST".to_string(),
            "https://user.example.com/".to_string(),
        );
        merged.insert("OTHER_KEY".to_string(), "other".to_string());

        let result = discovery_env_with_baked_floor(merged);

        assert_eq!(
            result.get("DATABRICKS_HOST").map(String::as_str),
            Some("https://user.example.com/"),
            "merged_env key must survive in discovery_env_with_baked_floor output"
        );
        assert_eq!(
            result.get("OTHER_KEY").map(String::as_str),
            Some("other"),
            "unrelated merged_env keys must pass through unchanged"
        );
    }
}
