//! Pure env-layer key resolution for card minting, split out of `card.rs`
//! to keep it under the desktop file-size ratchet. Wired via
//! `#[path = "card/key_layers.rs"] mod key_layers;` and re-exported.

use std::collections::BTreeMap;

/// Pure layering: global env < persona env < agent record env, then the
/// process environment as a development fallback. Returns the first
/// non-empty value for `key`.
pub(crate) fn resolve_env_from_layers(
    key: &str,
    global_env: &BTreeMap<String, String>,
    persona_env: &BTreeMap<String, String>,
    record_env: &BTreeMap<String, String>,
    process_value: Option<String>,
) -> Option<String> {
    for layer in [record_env, persona_env, global_env] {
        if let Some(v) = layer.get(key) {
            let v = v.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    process_value.filter(|k| !k.trim().is_empty())
}

/// Pure classification: same four env inputs as `resolve_env_from_layers`,
/// returns which layer supplies `OPENAI_API_KEY` (agent > persona > global >
/// process > none).
pub(crate) fn resolve_key_layer(
    global_env: &BTreeMap<String, String>,
    persona_env: &BTreeMap<String, String>,
    record_env: &BTreeMap<String, String>,
    process_value: Option<String>,
) -> &'static str {
    let key = "OPENAI_API_KEY";
    let nonempty = |m: &BTreeMap<String, String>| m.get(key).is_some_and(|v| !v.trim().is_empty());
    if nonempty(record_env) {
        return "agent";
    }
    if nonempty(persona_env) {
        return "persona";
    }
    if nonempty(global_env) {
        return "global";
    }
    let proc = process_value.as_deref().unwrap_or("");
    if !proc.trim().is_empty() {
        return "process";
    }
    "none"
}
