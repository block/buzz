use std::collections::BTreeMap;

const VERSIONS_JSON: &str = include_str!("schemas/versions.json");

/// Return the `fetched_at` timestamp for `harness` ("codex" or "claude") from
/// the embedded `versions.json`, or `None` if the entry is absent.
pub(super) fn schema_version(harness: &str) -> Option<String> {
    let versions: serde_json::Value = serde_json::from_str(VERSIONS_JSON).ok()?;
    versions
        .get(harness)
        .and_then(|v| v.get("fetched_at"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// Walk a JSON Schema's top-level `properties` and extract every key that is
/// present in `config`. Returns a flat `BTreeMap<String, String>` suitable for
/// `RuntimeFileConfig::extra`.
///
/// - Scalar values (string, number, bool) → their string representation
/// - Arrays → "[N items]"
/// - Objects → flatten one level deep as "key.subkey = value"; deeper nesting → "{...}".
///   Note: object subkeys are iterated from the config value, not filtered against the
///   schema's nested properties — so all subkeys the user has set are surfaced regardless
///   of whether the schema defines them (intentional: supports arbitrary keys like env vars).
/// - Keys in `skip` are excluded (used to avoid double-counting normalized fields)
pub(super) fn extract_schema_fields(
    schema_json: &str,
    config: &serde_json::Value,
    skip: &[&str],
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();

    let schema: serde_json::Value = match serde_json::from_str(schema_json) {
        Ok(v) => v,
        Err(_) => return out,
    };

    let properties = match schema.get("properties").and_then(|v| v.as_object()) {
        Some(p) => p,
        None => return out,
    };

    let config_obj = match config.as_object() {
        Some(o) => o,
        None => return out,
    };

    for key in properties.keys() {
        if skip.contains(&key.as_str()) {
            continue;
        }
        let value = match config_obj.get(key) {
            Some(v) => v,
            None => continue,
        };
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

    fn minimal_schema(props: serde_json::Value) -> String {
        json!({ "type": "object", "properties": props }).to_string()
    }

    #[test]
    fn extracts_scalar_string() {
        let schema = minimal_schema(json!({ "name": { "type": "string" } }));
        let config = json!({ "name": "alice" });
        let result = extract_schema_fields(&schema, &config, &[]);
        assert_eq!(result.get("name").map(|s| s.as_str()), Some("alice"));
    }

    #[test]
    fn extracts_scalar_bool() {
        let schema = minimal_schema(json!({ "enabled": { "type": "boolean" } }));
        let config = json!({ "enabled": true });
        let result = extract_schema_fields(&schema, &config, &[]);
        assert_eq!(result.get("enabled").map(|s| s.as_str()), Some("true"));
    }

    #[test]
    fn extracts_scalar_number() {
        let schema = minimal_schema(json!({ "count": { "type": "integer" } }));
        let config = json!({ "count": 42 });
        let result = extract_schema_fields(&schema, &config, &[]);
        assert_eq!(result.get("count").map(|s| s.as_str()), Some("42"));
    }

    #[test]
    fn formats_array_as_item_count() {
        let schema = minimal_schema(json!({ "tags": { "type": "array" } }));
        let config = json!({ "tags": ["a", "b", "c"] });
        let result = extract_schema_fields(&schema, &config, &[]);
        assert_eq!(result.get("tags").map(|s| s.as_str()), Some("[3 items]"));
    }

    #[test]
    fn flattens_object_one_level() {
        let schema = minimal_schema(json!({
            "env": {
                "type": "object",
                "properties": {
                    "FOO": { "type": "string" },
                    "BAR": { "type": "string" }
                }
            }
        }));
        let config = json!({ "env": { "FOO": "bar", "BAR": "baz" } });
        let result = extract_schema_fields(&schema, &config, &[]);
        assert_eq!(result.get("env.FOO").map(|s| s.as_str()), Some("bar"));
        assert_eq!(result.get("env.BAR").map(|s| s.as_str()), Some("baz"));
        assert!(!result.contains_key("env"));
    }

    #[test]
    fn nested_object_beyond_one_level_is_placeholder() {
        let schema = minimal_schema(json!({
            "nested": { "type": "object" }
        }));
        let config = json!({ "nested": { "deep": { "deeper": 1 } } });
        let result = extract_schema_fields(&schema, &config, &[]);
        // "nested.deep" should be "{...}" because the value is an object
        assert_eq!(
            result.get("nested.deep").map(|s| s.as_str()),
            Some("{...}")
        );
    }

    #[test]
    fn skip_list_excludes_keys() {
        let schema = minimal_schema(json!({
            "model": { "type": "string" },
            "extra": { "type": "string" }
        }));
        let config = json!({ "model": "gpt-4", "extra": "value" });
        let result = extract_schema_fields(&schema, &config, &["model"]);
        assert!(!result.contains_key("model"));
        assert!(result.contains_key("extra"));
    }

    #[test]
    fn key_in_schema_but_not_config_is_skipped() {
        let schema = minimal_schema(json!({
            "present": { "type": "string" },
            "absent": { "type": "string" }
        }));
        let config = json!({ "present": "yes" });
        let result = extract_schema_fields(&schema, &config, &[]);
        assert!(result.contains_key("present"));
        assert!(!result.contains_key("absent"));
    }

    #[test]
    fn key_in_config_but_not_schema_is_skipped() {
        let schema = minimal_schema(json!({ "known": { "type": "string" } }));
        let config = json!({ "known": "yes", "unknown": "no" });
        let result = extract_schema_fields(&schema, &config, &[]);
        assert!(result.contains_key("known"));
        assert!(!result.contains_key("unknown"));
    }

    #[test]
    fn invalid_schema_returns_empty() {
        let result = extract_schema_fields("not json", &json!({}), &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn schema_without_properties_returns_empty() {
        let schema = json!({ "type": "object" }).to_string();
        let result = extract_schema_fields(&schema, &json!({ "foo": "bar" }), &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn empty_array_formats_as_zero_items() {
        let schema = minimal_schema(json!({ "list": { "type": "array" } }));
        let config = json!({ "list": [] });
        let result = extract_schema_fields(&schema, &config, &[]);
        assert_eq!(result.get("list").map(|s| s.as_str()), Some("[0 items]"));
    }
}
