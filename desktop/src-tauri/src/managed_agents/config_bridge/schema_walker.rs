use std::collections::BTreeMap;

/// Walk a config JSON object and extract every key that is present, returning a
/// flat `BTreeMap<String, String>` suitable for `RuntimeFileConfig::extra`.
///
/// Keys in `skip` are excluded (used to avoid double-counting normalized fields
/// that are extracted into typed struct fields like `model`, `provider`, etc.).
///
/// Value formatting:
/// - Scalar values (string, number, bool) → their string representation
/// - Arrays → "[N items]"
/// - Objects → flatten up to two levels deep as "key.subkey" or
///   "key.subkey.subsubkey = value"; deeper nesting → "{...}".
///   Note: object subkeys are iterated from the config value, not filtered against the
///   schema's nested properties — so all subkeys the user has set are surfaced regardless
///   of whether the schema defines them (intentional: supports arbitrary keys like env vars).
pub(super) fn extract_config_fields(
    config: &serde_json::Value,
    skip: &[&str],
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();

    let config_obj = match config.as_object() {
        Some(o) => o,
        None => return out,
    };

    for (key, value) in config_obj {
        if skip.contains(&key.as_str()) {
            continue;
        }
        match value {
            serde_json::Value::Object(obj) => {
                // Flatten up to two levels: "key.subkey" and "key.subkey.subsubkey"
                for (subkey, subval) in obj {
                    let flat_key = format!("{key}.{subkey}");
                    match subval {
                        serde_json::Value::Object(subobj) => {
                            // Second level: "key.subkey.subsubkey"
                            if subobj.is_empty() {
                                // Empty inner object — emit placeholder so the key is visible.
                                out.insert(flat_key, "{...}".to_string());
                            } else {
                                for (subsubkey, subsubval) in subobj {
                                    let deep_key = format!("{flat_key}.{subsubkey}");
                                    out.insert(deep_key, format_scalar(subsubval));
                                }
                            }
                        }
                        serde_json::Value::Array(arr) => {
                            out.insert(flat_key, format_array(arr));
                        }
                        other => {
                            out.insert(flat_key, format_scalar(other));
                        }
                    }
                }
            }
            serde_json::Value::Array(arr) => {
                out.insert(key.clone(), format_array(arr));
            }
            other => {
                out.insert(key.clone(), format_scalar(other));
            }
        }
    }

    out
}

fn format_scalar(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(arr) => format_array(arr),
        serde_json::Value::Object(_) => "{...}".to_string(),
    }
}

/// Format an array for display.
///
/// When all elements are scalars (string, bool, number, null), joins them
/// comma-separated so the actual values are visible (e.g. `tui.status_line`).
/// When any element is a nested object or array, falls back to `[N items]`
/// since the values can't be meaningfully inlined.
fn format_array(arr: &[serde_json::Value]) -> String {
    let all_scalar = arr.iter().all(|v| {
        matches!(
            v,
            serde_json::Value::String(_)
                | serde_json::Value::Bool(_)
                | serde_json::Value::Number(_)
                | serde_json::Value::Null
        )
    });
    if all_scalar {
        arr.iter()
            .map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Bool(b) => b.to_string(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Null => "null".to_string(),
                _ => unreachable!(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        format!("[{} items]", arr.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_scalar_string() {
        let config = json!({ "name": "alice" });
        let result = extract_config_fields(&config, &[]);
        assert_eq!(result.get("name").map(|s| s.as_str()), Some("alice"));
    }

    #[test]
    fn extracts_scalar_bool() {
        let config = json!({ "enabled": true });
        let result = extract_config_fields(&config, &[]);
        assert_eq!(result.get("enabled").map(|s| s.as_str()), Some("true"));
    }

    #[test]
    fn extracts_scalar_number() {
        let config = json!({ "count": 42 });
        let result = extract_config_fields(&config, &[]);
        assert_eq!(result.get("count").map(|s| s.as_str()), Some("42"));
    }

    #[test]
    fn formats_scalar_array_as_joined_values() {
        let config = json!({ "tags": ["a", "b", "c"] });
        let result = extract_config_fields(&config, &[]);
        assert_eq!(result.get("tags").map(|s| s.as_str()), Some("a, b, c"));
    }

    #[test]
    fn flattens_object_one_level() {
        let config = json!({ "env": { "FOO": "bar", "BAR": "baz" } });
        let result = extract_config_fields(&config, &[]);
        assert_eq!(result.get("env.FOO").map(|s| s.as_str()), Some("bar"));
        assert_eq!(result.get("env.BAR").map(|s| s.as_str()), Some("baz"));
        assert!(!result.contains_key("env"));
    }

    #[test]
    fn empty_inner_object_emits_placeholder() {
        let config = json!({ "hooks": { "pre-commit": {} } });
        let result = extract_config_fields(&config, &[]);
        assert_eq!(
            result.get("hooks.pre-commit").map(|s| s.as_str()),
            Some("{...}")
        );
    }

    #[test]
    fn nested_object_at_two_levels_is_flattened() {
        let config = json!({ "nested": { "deep": { "value": 42 } } });
        let result = extract_config_fields(&config, &[]);
        // Two levels deep: "nested.deep.value" = "42"
        assert_eq!(
            result.get("nested.deep.value").map(|s| s.as_str()),
            Some("42")
        );
        assert!(!result.contains_key("nested.deep"));
    }

    #[test]
    fn nested_object_beyond_two_levels_is_placeholder() {
        let config = json!({ "a": { "b": { "c": { "d": 1 } } } });
        let result = extract_config_fields(&config, &[]);
        // "a.b.c" is depth 3 — value is an object → "{...}"
        assert_eq!(result.get("a.b.c").map(|s| s.as_str()), Some("{...}"));
    }

    #[test]
    fn projects_table_flattens_to_trust_level() {
        // Mirrors codex [projects."<path>"] { trust_level = "trusted" }
        let config = json!({
            "projects": {
                "/Users/foo/dev/buzz": { "trust_level": "trusted" },
                "/Users/foo/dev/other": { "trust_level": "untrusted" }
            }
        });
        let result = extract_config_fields(&config, &[]);
        assert_eq!(
            result
                .get("projects./Users/foo/dev/buzz.trust_level")
                .map(|s| s.as_str()),
            Some("trusted")
        );
        assert_eq!(
            result
                .get("projects./Users/foo/dev/other.trust_level")
                .map(|s| s.as_str()),
            Some("untrusted")
        );
    }

    #[test]
    fn tui_model_availability_nux_flattens_to_model_keys() {
        // Mirrors codex [tui.model_availability_nux] { "gpt-5.5" = 1 }
        let config = json!({
            "tui": {
                "status_line": ["model-with-reasoning", "git-branch"],
                "model_availability_nux": { "gpt-5.5": 1 }
            }
        });
        let result = extract_config_fields(&config, &[]);
        assert_eq!(
            result
                .get("tui.status_line")
                .map(|s| s.as_str()),
            Some("model-with-reasoning, git-branch")
        );
        assert_eq!(
            result
                .get("tui.model_availability_nux.gpt-5.5")
                .map(|s| s.as_str()),
            Some("1")
        );
        assert!(!result.contains_key("tui.model_availability_nux"));
    }

    #[test]
    fn skip_list_excludes_keys() {
        let config = json!({ "model": "gpt-4", "extra": "value" });
        let result = extract_config_fields(&config, &["model"]);
        assert!(!result.contains_key("model"));
        assert!(result.contains_key("extra"));
    }

    #[test]
    fn unknown_keys_are_surfaced() {
        // Config-driven: any key the user has set appears, no schema gate.
        let config = json!({ "known": "yes", "unknown_future_field": "also yes" });
        let result = extract_config_fields(&config, &[]);
        assert!(result.contains_key("known"));
        assert!(result.contains_key("unknown_future_field"));
    }

    #[test]
    fn empty_config_returns_empty() {
        let result = extract_config_fields(&json!({}), &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn non_object_config_returns_empty() {
        let result = extract_config_fields(&json!("not an object"), &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn empty_array_formats_as_empty_string() {
        let config = json!({ "list": [] });
        let result = extract_config_fields(&config, &[]);
        // Empty array: all-scalar vacuously → join produces empty string
        assert_eq!(result.get("list").map(|s| s.as_str()), Some(""));
    }

    #[test]
    fn array_with_nested_objects_falls_back_to_item_count() {
        let config = json!({ "items": [{"key": "val"}, {"key": "val2"}] });
        let result = extract_config_fields(&config, &[]);
        assert_eq!(result.get("items").map(|s| s.as_str()), Some("[2 items]"));
    }

    #[test]
    fn arbitrary_env_subkeys_surfaced_without_schema() {
        // Env vars are arbitrary strings — all should appear regardless of whether
        // any schema defines them.
        let config = json!({ "env": { "MY_CUSTOM_VAR": "hello", "ANOTHER_VAR": "world" } });
        let result = extract_config_fields(&config, &[]);
        assert_eq!(
            result.get("env.MY_CUSTOM_VAR").map(|s| s.as_str()),
            Some("hello")
        );
        assert_eq!(
            result.get("env.ANOTHER_VAR").map(|s| s.as_str()),
            Some("world")
        );
    }
}
