//! Build-time agent env passthrough.
//!
//! Internal builds (buzz-releases) bake arbitrary `KEY=VALUE` pairs into the
//! binary via `BUZZ_BUILD_AGENT_ENV` (base64-encoded, newline-delimited).
//! OSS builds leave the compile-time var unset — nothing is injected.

use base64::Engine as _;

/// Inject baked-in provider/model defaults and generic env pairs onto `cmd`.
///
/// Call this BEFORE writing record/persona metadata env vars so that the
/// record's explicit choices (written after) override the baked defaults.
/// User-supplied `record.env_vars` (written last) always win.
pub(crate) fn build_buzz_agent_provider_defaults(cmd: &mut std::process::Command) {
    if let Some(provider) = option_env!("BUZZ_DESKTOP_BUILD_BUZZ_AGENT_PROVIDER") {
        if !provider.is_empty() {
            cmd.env("BUZZ_AGENT_PROVIDER", provider);
        }
    }
    if let Some(model) = option_env!("BUZZ_DESKTOP_BUILD_BUZZ_AGENT_MODEL") {
        if !model.is_empty() {
            cmd.env("BUZZ_AGENT_MODEL", model);
        }
    }
    if let Some(raw) = option_env!("BUZZ_DESKTOP_BUILD_AGENT_ENV") {
        // The value was base64-encoded at build time so the single-line Cargo
        // output carries all KEY=VALUE pairs without truncation.
        if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(raw.as_bytes()) {
            if let Ok(text) = std::str::from_utf8(&decoded) {
                for (key, value) in parse_agent_env_lines(text) {
                    cmd.env(key, value);
                }
            }
        }
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
    use super::{build_buzz_agent_provider_defaults, parse_agent_env_lines};

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
}
