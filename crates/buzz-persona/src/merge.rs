/// Precedence resolution for persona behavioral config.
///
/// Handles levels 3–5 of the 5-level precedence model:
///   3. Per-persona frontmatter  (wins)
///   4. Pack-level defaults      (from plugin.json `defaults`)
///   5. Built-in defaults        (hardcoded fallbacks)
///
/// Levels 1–2 (operator env vars, desktop UI) are resolved at runtime.

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct TriggersData {
    pub mentions: bool,
    pub keywords: Vec<String>,
    pub all_messages: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HooksData {
    pub on_start: Option<String>,
    pub on_stop: Option<String>,
    pub on_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedConfig {
    pub model: Option<String>,
    pub temperature: Option<f64>,
    pub max_context_tokens: Option<u64>,
    /// `None`       = absent (no persona or pack value) → caller uses its own default
    /// `Some([])`   = intentional "subscribe to nothing"
    /// `Some([..])` = explicit channel list
    pub subscribe: Option<Vec<String>>,
    pub triggers: Option<TriggersData>,
    pub thread_replies: bool,
    pub broadcast_replies: bool,
}

// ── Built-in defaults ─────────────────────────────────────────────────────────

const DEFAULT_THREAD_REPLIES: bool = true;
const DEFAULT_BROADCAST_REPLIES: bool = false;

// ── Core merge ────────────────────────────────────────────────────────────────

/// Merge pack defaults with per-persona values.
///
/// Rules:
/// - Persona field present (non-null) → use persona value
/// - Persona field absent or null     → use pack default
/// - Empty array `[]` or object `{}`  → present, overrides default
pub fn merge_behavioral_config(
    persona_config: &serde_json::Value,
    pack_defaults: &serde_json::Value,
) -> serde_json::Value {
    use serde_json::Value;

    let persona_obj = match persona_config.as_object() {
        Some(o) => o,
        None => return pack_defaults.clone(),
    };
    let defaults_obj = match pack_defaults.as_object() {
        Some(o) => o,
        None => return persona_config.clone(),
    };

    let mut merged = serde_json::Map::new();

    // All keys from defaults first, then persona overrides.
    for (key, default_val) in defaults_obj {
        let effective = match persona_obj.get(key) {
            // null in persona → fall through to default
            Some(Value::Null) | None => default_val.clone(),
            Some(v) => v.clone(),
        };
        merged.insert(key.clone(), effective);
    }

    // Any persona keys not in defaults are included as-is (excluding null).
    for (key, val) in persona_obj {
        if !merged.contains_key(key) && !val.is_null() {
            merged.insert(key.clone(), val.clone());
        }
    }

    Value::Object(merged)
}

// ── High-level resolver ───────────────────────────────────────────────────────

/// Resolve a single persona's effective config from raw frontmatter + pack defaults.
pub fn resolve_persona_config(
    persona_frontmatter: &serde_json::Value,
    pack_defaults: Option<&serde_json::Value>,
) -> ResolvedConfig {
    let empty = serde_json::Value::Object(serde_json::Map::new());
    let defaults = pack_defaults.unwrap_or(&empty);
    let merged = merge_behavioral_config(persona_frontmatter, defaults);

    let model = string_field(&merged, "model");
    let temperature = merged.get("temperature").and_then(|v| v.as_f64());
    let max_context_tokens = merged.get("max_context_tokens").and_then(|v| v.as_u64());

    // subscribe: Option<Vec<String>>
    //
    // Read from persona frontmatter first to distinguish:
    //   - absent / null  → None (fall through to pack default, then None)
    //   - Some([])       → intentional "subscribe to nothing"
    //   - Some([..])     → explicit channel list
    //
    // Pack default is only used when persona has no subscribe (None/null).
    let subscribe = {
        let persona_sub = persona_frontmatter.get("subscribe");
        let default_sub = defaults.get("subscribe");
        match (persona_sub, default_sub) {
            // Persona has a non-null array → use it directly (may be empty).
            (Some(serde_json::Value::Array(arr)), _) => Some(
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
            ),
            // Persona absent or null → try pack default.
            (None, Some(serde_json::Value::Array(arr)))
            | (Some(serde_json::Value::Null), Some(serde_json::Value::Array(arr))) => Some(
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
            ),
            // Neither side has subscribe.
            _ => None,
        }
    };

    // triggers: SHALLOW REPLACEMENT.
    //
    // If persona has triggers (non-null), it replaces the pack default entirely.
    // Missing sub-fields fall to BUILT-IN defaults, not pack defaults.
    //
    // If persona lacks triggers (null or absent), use pack default.
    // If neither has it, None.
    let triggers = {
        let persona_t = persona_frontmatter.get("triggers");
        let default_t = defaults.get("triggers");
        match (persona_t, default_t) {
            // Neither side has triggers.
            (None, None)
            | (Some(serde_json::Value::Null), None)
            | (None, Some(serde_json::Value::Null))
            | (Some(serde_json::Value::Null), Some(serde_json::Value::Null)) => None,
            // Persona has triggers — use it directly (shallow replacement).
            // Pack default is ignored entirely.
            (Some(v), _) if !v.is_null() => parse_triggers(v),
            // Persona absent/null — fall through to pack default.
            (None, Some(v)) | (Some(serde_json::Value::Null), Some(v)) => parse_triggers(v),
            _ => None,
        }
    };

    let thread_replies = merged
        .get("thread_replies")
        .and_then(|v| v.as_bool())
        .unwrap_or(DEFAULT_THREAD_REPLIES);

    let broadcast_replies = merged
        .get("broadcast_replies")
        .and_then(|v| v.as_bool())
        .unwrap_or(DEFAULT_BROADCAST_REPLIES);

    ResolvedConfig {
        model,
        temperature,
        max_context_tokens,
        subscribe,
        triggers,
        thread_replies,
        broadcast_replies,
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn string_field(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key).and_then(|v| v.as_str()).map(str::to_owned)
}

fn parse_triggers(v: &serde_json::Value) -> Option<TriggersData> {
    let obj = v.as_object()?;
    Some(TriggersData {
        mentions: obj
            .get("mentions")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        keywords: obj
            .get("keywords")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        all_messages: obj
            .get("all_messages")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── merge_behavioral_config ───────────────────────────────────────────────

    #[test]
    fn persona_value_wins_over_pack_default() {
        let persona = json!({ "model": "gpt-4o", "thread_replies": false });
        let defaults = json!({ "model": "claude-3", "thread_replies": true });
        let merged = merge_behavioral_config(&persona, &defaults);
        assert_eq!(merged["model"], "gpt-4o");
        assert_eq!(merged["thread_replies"], false);
    }

    #[test]
    fn pack_default_used_when_persona_field_absent() {
        let persona = json!({ "model": "gpt-4o" });
        let defaults = json!({ "model": "claude-3", "temperature": 0.7 });
        let merged = merge_behavioral_config(&persona, &defaults);
        assert_eq!(merged["model"], "gpt-4o");
        assert_eq!(merged["temperature"], 0.7);
    }

    #[test]
    fn null_persona_value_falls_through_to_default() {
        let persona = json!({ "model": null });
        let defaults = json!({ "model": "claude-3" });
        let merged = merge_behavioral_config(&persona, &defaults);
        assert_eq!(merged["model"], "claude-3");
    }

    #[test]
    fn empty_array_overrides_default() {
        let persona = json!({ "subscribe": [] });
        let defaults = json!({ "subscribe": ["channel-a", "channel-b"] });
        let merged = merge_behavioral_config(&persona, &defaults);
        assert_eq!(merged["subscribe"], json!([]));
    }

    #[test]
    fn empty_object_overrides_default() {
        let persona = json!({ "triggers": {} });
        let defaults = json!({ "triggers": { "mentions": true } });
        let merged = merge_behavioral_config(&persona, &defaults);
        assert_eq!(merged["triggers"], json!({}));
    }

    #[test]
    fn no_defaults_persona_fields_pass_through() {
        let persona = json!({ "model": "gpt-4o", "temperature": 0.5 });
        let defaults = json!({});
        let merged = merge_behavioral_config(&persona, &defaults);
        assert_eq!(merged["model"], "gpt-4o");
        assert_eq!(merged["temperature"], 0.5);
    }

    #[test]
    fn full_merge_mixed_present_absent() {
        let persona = json!({
            "model": "gpt-4o",
            "temperature": null,
            "subscribe": ["chan-x"],
        });
        let defaults = json!({
            "model": "claude-3",
            "temperature": 0.9,
            "thread_replies": false,
            "subscribe": ["chan-default"],
        });
        let merged = merge_behavioral_config(&persona, &defaults);
        assert_eq!(merged["model"], "gpt-4o"); // persona wins
        assert_eq!(merged["temperature"], 0.9); // null → default
        assert_eq!(merged["thread_replies"], false); // default used
        assert_eq!(merged["subscribe"], json!(["chan-x"])); // persona wins
    }

    // ── resolve_persona_config ────────────────────────────────────────────────

    #[test]
    fn built_in_defaults_when_no_fields() {
        let persona = json!({});
        let resolved = resolve_persona_config(&persona, None);
        assert_eq!(resolved.model, None);
        assert_eq!(resolved.temperature, None);
        assert_eq!(resolved.max_context_tokens, None);
        assert_eq!(resolved.subscribe, None);
        assert_eq!(resolved.triggers, None);
        assert!(resolved.thread_replies); // built-in default
        assert!(!resolved.broadcast_replies); // built-in default
    }

    #[test]
    fn resolve_with_pack_defaults() {
        let persona = json!({ "model": "gpt-4o" });
        let defaults = json!({
            "temperature": 0.7,
            "thread_replies": false,
            "subscribe": ["general"],
        });
        let resolved = resolve_persona_config(&persona, Some(&defaults));
        assert_eq!(resolved.model.as_deref(), Some("gpt-4o"));
        assert_eq!(resolved.temperature, Some(0.7));
        assert!(!resolved.thread_replies);
        assert_eq!(resolved.subscribe, Some(vec!["general".to_owned()]));
    }

    #[test]
    fn resolve_triggers_parsed() {
        let persona = json!({
            "triggers": {
                "mentions": true,
                "keywords": ["help", "sprout"],
                "all_messages": false,
            }
        });
        let resolved = resolve_persona_config(&persona, None);
        let t = resolved.triggers.unwrap();
        assert!(t.mentions);
        assert_eq!(t.keywords, vec!["help", "sprout"]);
        assert!(!t.all_messages);
    }

    #[test]
    fn resolve_max_context_tokens() {
        let persona = json!({ "max_context_tokens": 8192u64 });
        let resolved = resolve_persona_config(&persona, None);
        assert_eq!(resolved.max_context_tokens, Some(8192));
    }

    // ── triggers shallow replacement ─────────────────────────────────────────

    #[test]
    fn triggers_shallow_replacement() {
        // Persona sets `mentions: false` — entire triggers replaces pack default.
        // Pack's `keywords: ["security"]` is LOST. Missing sub-fields get built-in defaults.
        let persona = json!({ "triggers": { "mentions": false } });
        let defaults = json!({ "triggers": { "mentions": true, "keywords": ["security"] } });
        let resolved = resolve_persona_config(&persona, Some(&defaults));
        let t = resolved.triggers.unwrap();
        assert!(!t.mentions);
        assert!(
            t.keywords.is_empty(),
            "pack keywords should be lost under shallow replacement"
        );
        assert!(!t.all_messages); // built-in default
    }

    #[test]
    fn triggers_absent_inherits_pack() {
        // Persona has no triggers → pack default used entirely.
        let persona = json!({ "model": "gpt-4o" });
        let defaults = json!({ "triggers": { "mentions": false, "keywords": ["deploy"] } });
        let resolved = resolve_persona_config(&persona, Some(&defaults));
        let t = resolved.triggers.unwrap();
        assert!(!t.mentions);
        assert_eq!(t.keywords, vec!["deploy"]);
    }

    #[test]
    fn triggers_empty_gets_builtins() {
        // Persona sets `triggers: {}` — present but empty.
        // All sub-fields fall to built-in defaults.
        let persona = json!({ "triggers": {} });
        let defaults = json!({ "triggers": { "mentions": false, "keywords": ["security"] } });
        let resolved = resolve_persona_config(&persona, Some(&defaults));
        let t = resolved.triggers.unwrap();
        assert!(t.mentions, "built-in default for mentions is true");
        assert!(
            t.keywords.is_empty(),
            "built-in default for keywords is empty"
        );
        assert!(
            !t.all_messages,
            "built-in default for all_messages is false"
        );
    }

    #[test]
    fn triggers_null_inherits_pack_default() {
        // Persona explicitly sets triggers: null — fall through to pack default.
        let persona = json!({ "triggers": null });
        let defaults = json!({ "triggers": { "all_messages": true } });
        let resolved = resolve_persona_config(&persona, Some(&defaults));
        let t = resolved.triggers.unwrap();
        assert!(t.all_messages);
    }

    #[test]
    fn triggers_persona_explicit_empty_keywords_overrides_pack() {
        // Persona explicitly sets `keywords: []`; pack default has `keywords: ["foo"]`.
        // Under shallow replacement, persona wins entirely — pack is ignored.
        let persona = json!({ "triggers": { "keywords": [] } });
        let defaults = json!({ "triggers": { "keywords": ["foo"] } });
        let resolved = resolve_persona_config(&persona, Some(&defaults));
        let t = resolved.triggers.unwrap();
        assert!(t.keywords.is_empty());
    }

    #[test]
    fn triggers_neither_side_returns_none() {
        // Neither persona nor pack has triggers — result is None.
        let persona = json!({ "model": "gpt-4o" });
        let resolved = resolve_persona_config(&persona, None);
        assert!(resolved.triggers.is_none());
    }

    #[test]
    fn triggers_inherited_from_pack_defaults() {
        // Critical regression test: a persona WITHOUT triggers must inherit
        // pack-default triggers. This was broken when BehavioralDefaults
        // serialized the field as "respond_to" but merge looked for "triggers".
        let persona = json!({ "model": "gpt-4o" });
        let defaults = json!({
            "triggers": {
                "mentions": true,
                "keywords": ["security", "CVE"],
                "all_messages": false,
            }
        });
        let resolved = resolve_persona_config(&persona, Some(&defaults));
        let t = resolved
            .triggers
            .expect("persona should inherit triggers from pack defaults");
        assert!(t.mentions);
        assert_eq!(t.keywords, vec!["security", "CVE"]);
        assert!(!t.all_messages);
    }

    // ── subscribe merge (Option<Vec<String>>) ────────────────────────────────

    #[test]
    fn subscribe_null_falls_through() {
        // persona subscribe: null → falls through to pack default.
        let persona = json!({ "subscribe": null });
        let defaults = json!({ "subscribe": ["#general", "#alerts"] });
        let resolved = resolve_persona_config(&persona, Some(&defaults));
        assert_eq!(
            resolved.subscribe,
            Some(vec!["#general".to_owned(), "#alerts".to_owned()])
        );
    }

    #[test]
    fn subscribe_empty_overrides() {
        // persona subscribe: [] → intentional "subscribe to nothing" — Some([]).
        let persona = json!({ "subscribe": [] });
        let defaults = json!({ "subscribe": ["#general"] });
        let resolved = resolve_persona_config(&persona, Some(&defaults));
        assert_eq!(resolved.subscribe, Some(vec![]));
    }

    #[test]
    fn subscribe_absent_falls_through_to_pack_default() {
        // persona has no subscribe field → falls through to pack default.
        let persona = json!({ "model": "gpt-4o" });
        let defaults = json!({ "subscribe": ["#security-reviews"] });
        let resolved = resolve_persona_config(&persona, Some(&defaults));
        assert_eq!(
            resolved.subscribe,
            Some(vec!["#security-reviews".to_owned()])
        );
    }

    #[test]
    fn subscribe_absent_no_pack_is_none() {
        // Neither persona nor pack has subscribe → None.
        let persona = json!({ "model": "gpt-4o" });
        let resolved = resolve_persona_config(&persona, None);
        assert_eq!(resolved.subscribe, None);
    }
}
