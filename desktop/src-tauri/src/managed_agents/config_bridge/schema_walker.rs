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
/// - Objects → flatten one level deep as "key.subkey = value"; deeper nesting → "{...}".
///   Subkeys are iterated from the config value directly — all subkeys the user
///   has set are surfaced regardless of whether any schema defines them (intentional:
///   supports arbitrary keys like env vars and custom plugin entries).
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
                // Flatten one level: "key.subkey" = scalar_value
                for (subkey, subval) in obj {
                    let flat_key = format!("{key}.{subkey}");
                    out.insert(flat_key, format_scalar(subval));
                }
            }
            serde_json::Value::Array(arr) => {
                out.insert(key.clone(), format!("[{} items]", arr.len()));
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
        serde_json::Value::Array(arr) => format!("[{} items]", arr.len()),
        serde_json::Value::Object(_) => "{...}".to_string(),
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
    fn formats_array_as_item_count() {
        let config = json!({ "tags": ["a", "b", "c"] });
        let result = extract_config_fields(&config, &[]);
        assert_eq!(result.get("tags").map(|s| s.as_str()), Some("[3 items]"));
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
    fn nested_object_beyond_one_level_is_placeholder() {
        let config = json!({ "nested": { "deep": { "deeper": 1 } } });
        let result = extract_config_fields(&config, &[]);
        // "nested.deep" should be "{...}" because the value is an object
        assert_eq!(
            result.get("nested.deep").map(|s| s.as_str()),
            Some("{...}")
        );
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
    fn empty_array_formats_as_zero_items() {
        let config = json!({ "list": [] });
        let result = extract_config_fields(&config, &[]);
        assert_eq!(result.get("list").map(|s| s.as_str()), Some("[0 items]"));
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
