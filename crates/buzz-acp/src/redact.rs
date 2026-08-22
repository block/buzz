//! Redact secret-shaped values from ACP JSON before logging or observer relay.
//!
//! Agents sometimes rebroadcast MCP server configs (including plaintext env
//! values such as API keys) in custom notifications. Wire debug logs and the
//! observer feed must not persist those values.
//!
//! The agent stdin/stdout path is never mutated — only copies used for
//! `tracing` / `observe` are scrubbed.

use serde_json::{Map, Value};

const REDACTED: &str = "[REDACTED]";

/// Words that mark a key as secret-shaped when they appear as a segment.
///
/// Matches `ANTHROPIC_API_KEY`, `apiKey`, `access-token`, `clientSecret`, etc.
/// Does not match incidental words like `keyboard` (no forbidden segment).
const FORBIDDEN_SEGMENTS: &[&str] = &["secret", "password", "token", "key", "credential"];

/// Return a deep copy of `value` with secret-shaped string/number values replaced.
pub fn redact_secret_shaped_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(redact_object(map)),
        Value::Array(items) => Value::Array(items.iter().map(redact_secret_shaped_json).collect()),
        other => other.clone(),
    }
}

/// Best-effort redact for a raw NDJSON wire line (used in `acp::wire` logs).
pub fn redact_wire_line(line: &str) -> String {
    match serde_json::from_str::<Value>(line) {
        Ok(v) => serde_json::to_string(&redact_secret_shaped_json(&v)).unwrap_or_else(|_| {
            // Serialization failure is extremely unlikely for a Value we just
            // parsed; fall back without leaking the original line.
            REDACTED.to_string()
        }),
        // Non-JSON noise: leave unchanged (no structured secrets to scrub).
        Err(_) => line.to_string(),
    }
}

fn redact_object(map: &Map<String, Value>) -> Map<String, Value> {
    // ACP MCP env entries are `{ "name": "ANTHROPIC_API_KEY", "value": "…" }`.
    // Scrub `value` when `name` is secret-shaped.
    let env_pair_secret = map
        .get("name")
        .and_then(|v| v.as_str())
        .is_some_and(is_secret_shaped_key)
        && map.contains_key("value");

    let mut out = Map::with_capacity(map.len());
    for (key, val) in map {
        if key == "mcpServers" {
            out.insert(key.clone(), redact_mcp_servers(val));
            continue;
        }
        if env_pair_secret && key == "value" {
            out.insert(key.clone(), Value::String(REDACTED.to_string()));
            continue;
        }
        if is_secret_shaped_key(key) && is_scalar_secret_value(val) {
            out.insert(key.clone(), Value::String(REDACTED.to_string()));
            continue;
        }
        out.insert(key.clone(), redact_secret_shaped_json(val));
    }
    out
}

/// MCP server environment values are configuration, not display data. Redact
/// them all, irrespective of the variable name (for example, `DATABASE_URL`).
fn redact_mcp_servers(value: &Value) -> Value {
    match value {
        Value::Array(servers) => Value::Array(
            servers
                .iter()
                .map(|server| match server {
                    Value::Object(map) => {
                        let mut out = Map::with_capacity(map.len());
                        for (key, val) in map {
                            if key == "env" {
                                out.insert(key.clone(), redact_mcp_env(val));
                            } else {
                                out.insert(key.clone(), redact_secret_shaped_json(val));
                            }
                        }
                        Value::Object(out)
                    }
                    other => redact_secret_shaped_json(other),
                })
                .collect(),
        ),
        other => redact_secret_shaped_json(other),
    }
}

fn redact_mcp_env(value: &Value) -> Value {
    match value {
        Value::Object(entries) => Value::Object(
            entries
                .keys()
                .map(|key| (key.clone(), Value::String(REDACTED.to_string())))
                .collect(),
        ),
        Value::Array(entries) => Value::Array(
            entries
                .iter()
                .map(|entry| match entry {
                    Value::Object(pair) => {
                        let mut out = Map::with_capacity(pair.len());
                        for (key, val) in pair {
                            if key == "name" {
                                out.insert(key.clone(), val.clone());
                            } else {
                                out.insert(key.clone(), Value::String(REDACTED.to_string()));
                            }
                        }
                        Value::Object(out)
                    }
                    _ => Value::String(REDACTED.to_string()),
                })
                .collect(),
        ),
        _ => Value::String(REDACTED.to_string()),
    }
}

fn is_scalar_secret_value(val: &Value) -> bool {
    matches!(val, Value::String(_) | Value::Number(_) | Value::Bool(_))
}

fn is_secret_shaped_key(key: &str) -> bool {
    let words = split_config_key(key);
    FORBIDDEN_SEGMENTS
        .iter()
        .any(|f| words.iter().any(|w| w == f))
}

/// Split on separators and camelCase / acronym boundaries.
///
/// Mirrors the desktop `split_config_key` heuristic so the same keys are
/// treated as secret-shaped on both sides of the stack.
fn split_config_key(key: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = key.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        if ch == '_' || ch == '-' || ch == '.' {
            if !current.is_empty() {
                words.push(current.to_lowercase());
                current.clear();
            }
        } else if ch.is_uppercase() {
            let prev_lower =
                !current.is_empty() && current.chars().last().is_some_and(|c| c.is_lowercase());
            let acronym_end = !current.is_empty()
                && current.chars().last().is_some_and(|c| c.is_uppercase())
                && chars.get(i + 1).is_some_and(|c| c.is_lowercase());
            if prev_lower || acronym_end {
                words.push(current.to_lowercase());
                current.clear();
            }
            current.push(ch);
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        words.push(current.to_lowercase());
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_object_env_map_api_key() {
        let input = json!({
            "servers": [{
                "name": "search",
                "env": {
                    "ANTHROPIC_API_KEY": "sk-secret-value",
                    "REGION": "us-east-1"
                }
            }]
        });
        let out = redact_secret_shaped_json(&input);
        assert_eq!(out["servers"][0]["env"]["ANTHROPIC_API_KEY"], REDACTED);
        assert_eq!(out["servers"][0]["env"]["REGION"], "us-east-1");
    }

    #[test]
    fn redacts_acp_env_name_value_pairs() {
        let input = json!({
            "method": "_x.ai/mcp/servers_updated",
            "params": {
                "mcpServers": [{
                    "name": "tools",
                    "command": "uvx",
                    "env": [
                        {"name": "OPENAI_API_KEY", "value": "sk-live-abc"},
                        {"name": "HOME", "value": "/tmp"}
                    ]
                }]
            }
        });
        let out = redact_secret_shaped_json(&input);
        let env = &out["params"]["mcpServers"][0]["env"];
        assert_eq!(env[0]["name"], "OPENAI_API_KEY");
        assert_eq!(env[0]["value"], REDACTED);
        assert_eq!(env[1]["name"], "HOME");
        assert_eq!(env[1]["value"], REDACTED);
    }

    #[test]
    fn redacts_all_mcp_server_env_map_values() {
        let input = json!({
            "params": {
                "mcpServers": [{
                    "env": {
                        "DATABASE_URL": "postgres://user:password@host/database",
                        "REGION": "us-east-1"
                    }
                }]
            },
            "env": {"REGION": "us-east-1"}
        });
        let out = redact_secret_shaped_json(&input);
        let env = &out["params"]["mcpServers"][0]["env"];
        assert_eq!(env["DATABASE_URL"], REDACTED);
        assert_eq!(env["REGION"], REDACTED);
        assert_eq!(out["env"]["REGION"], "us-east-1");
    }

    #[test]
    fn redacts_camel_case_and_token_suffixes() {
        let input = json!({
            "apiKey": "abc",
            "access_token": "tok",
            "client-secret": "shh",
            "keyboard": "qwerty"
        });
        let out = redact_secret_shaped_json(&input);
        assert_eq!(out["apiKey"], REDACTED);
        assert_eq!(out["access_token"], REDACTED);
        assert_eq!(out["client-secret"], REDACTED);
        assert_eq!(out["keyboard"], "qwerty");
    }

    #[test]
    fn redact_wire_line_scrubs_json_keeps_noise() {
        let line = r#"{"env":{"FOO_TOKEN":"leak-me"}}"#;
        let scrubbed = redact_wire_line(line);
        assert!(!scrubbed.contains("leak-me"));
        assert!(scrubbed.contains(REDACTED));
        assert_eq!(redact_wire_line("not-json"), "not-json");
    }

    #[test]
    fn split_config_key_handles_styles() {
        assert_eq!(split_config_key("apiKey"), vec!["api", "key"]);
        assert_eq!(split_config_key("ANTHROPIC_API_KEY"), vec!["anthropic", "api", "key"]);
        assert_eq!(split_config_key("keyboard"), vec!["keyboard"]);
    }
}
