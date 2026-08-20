pub const RELAY_MESH_API_BASE_URL: &str = "http://127.0.0.1:9337/v1";
pub const RELAY_MESH_API_KEY_PLACEHOLDER: &str = "buzz-mesh-local";
pub const RELAY_MESH_PROVIDER_ID: &str = "relay-mesh";
/// Stored value for "let the mesh decide", kept as the user-facing word.
pub const RELAY_MESH_AUTO_MODEL_ID: &str = "auto";
/// MeshLLM's virtual model. It resolves per request: a Mixture-of-Agents
/// committee when two or more workers are reachable, and otherwise degrades to
/// a single served model rather than erroring
/// (`moa_gateway::degrade_to_single_model`). That degradation is a pre-flight
/// capacity decision, so a committee that forms and *then* loses a worker still
/// surfaces as a failed turn — MoA repairs partial results internally
/// (`repair_tool_result_answer`) before it gets that far. Buzz translates the
/// stored `auto` here rather than teaching buzz-agent anything about meshes.
#[cfg(feature = "mesh-llm")]
pub const RELAY_MESH_VIRTUAL_MODEL_ID: &str = "mesh";

/// The wire name for a stored shared-compute model: `auto` (and a blank legacy
/// value) means "let the mesh decide" and becomes MeshLLM's virtual `mesh`
/// model; anything else is a model the user named and is passed through.
///
/// The single place this mapping happens. Every consumer that has to name a
/// model to the mesh — the LLM transport env and the ACP harness — goes through
/// here, so they cannot disagree.
#[cfg(feature = "mesh-llm")]
pub fn relay_mesh_wire_model(stored: &str) -> &str {
    match stored.trim() {
        "" | RELAY_MESH_AUTO_MODEL_ID => RELAY_MESH_VIRTUAL_MODEL_ID,
        named => named,
    }
}

/// Translate the native Buzz shared compute provider into the OpenAI-compatible
/// transport understood by buzz-agent. These are derived runtime details, not
/// user-owned agent configuration.
#[cfg(feature = "mesh-llm")]
pub fn apply_relay_mesh_env(
    env: &mut std::collections::BTreeMap<String, String>,
    provider: Option<&str>,
    model: Option<&str>,
) {
    if provider.map(str::trim) != Some(RELAY_MESH_PROVIDER_ID) {
        return;
    }
    let model = relay_mesh_wire_model(model.unwrap_or(RELAY_MESH_AUTO_MODEL_ID)).to_string();
    env.insert("BUZZ_AGENT_PROVIDER".to_string(), "openai".to_string());
    env.insert("BUZZ_AGENT_MODEL".to_string(), model.clone());
    env.insert(
        "OPENAI_COMPAT_BASE_URL".to_string(),
        RELAY_MESH_API_BASE_URL.to_string(),
    );
    env.insert("OPENAI_COMPAT_MODEL".to_string(), model);
    env.insert(
        "OPENAI_COMPAT_API_KEY".to_string(),
        RELAY_MESH_API_KEY_PLACEHOLDER.to_string(),
    );
    env.insert("OPENAI_COMPAT_API".to_string(), "chat".to_string());
    // Keep the requested response inside smaller local-model context windows.
    // These are defaults, not policy: the effective agent/persona/global env
    // may deliberately choose a smaller cap or a different effort. This function
    // runs after those layers during readiness, so never clobber their values.
    insert_default_if_unset(env, "BUZZ_AGENT_MAX_OUTPUT_TOKENS", "4096");
    // Mesh agents run on small local models, which are the ones most likely to
    // do the work and then end the turn without publishing it — the failure the
    // reply guard exists to catch. Everywhere else it stays opt-in and unset.
    // A default, not policy: an explicit `0` from the agent/persona/global env
    // survives (see `insert_default_if_unset`, and the copy-forward list in
    // `relay_mesh_process_env` that preserves it through the spawn path).
    insert_default_if_unset(env, "BUZZ_AGENT_REQUIRE_REPLY", "1");
    // Deliberately no BUZZ_AGENT_THINKING_EFFORT default: mesh translates
    // `reasoning_effort` into the chat template's `enable_thinking` flag, so any
    // value we pick overrides each model's own template default — and the right
    // value is model-specific. Measured with the real prompt and toolset:
    // gemma-4-E4B delivers 0/8 at `none` but 6/6 with the field absent, while
    // Qwen3-8B delivers 8/8 either way and burns ~4x the output tokens once
    // thinking is on (121 -> ~470), risking the 4096 cap. Omitting the field
    // lets every model use its own default; explicit agent/persona/global
    // values still apply.
}

#[cfg(feature = "mesh-llm")]
fn insert_default_if_unset(
    env: &mut std::collections::BTreeMap<String, String>,
    key: &str,
    value: &str,
) {
    if env.get(key).is_none_or(|current| current.trim().is_empty()) {
        env.insert(key.to_string(), value.to_string());
    }
}

/// Build the final Mesh-specific process overrides from the already-resolved
/// harness environment. Only user-owned generation controls are seeded: the
/// derived provider/base URL/model values remain authoritative, and unrelated
/// credentials (notably `OPENAI_API_KEY`) must not be copied back after the
/// spawn path removes them.
#[cfg(feature = "mesh-llm")]
pub fn relay_mesh_process_env(
    effective_env: &std::collections::BTreeMap<String, String>,
    model: &str,
) -> std::collections::BTreeMap<String, String> {
    let mut env = std::collections::BTreeMap::new();
    for key in [
        "BUZZ_AGENT_MAX_OUTPUT_TOKENS",
        "BUZZ_AGENT_THINKING_EFFORT",
        // Must be copied forward for the user's value to survive: this map is
        // written onto the command *after* the layered user env, so a key absent
        // here is re-defaulted by `apply_relay_mesh_env` below and an explicit
        // `BUZZ_AGENT_REQUIRE_REPLY=0` would be silently overridden back to `1`.
        "BUZZ_AGENT_REQUIRE_REPLY",
    ] {
        if let Some(value) = effective_env.get(key) {
            env.insert(key.to_string(), value.clone());
        }
    }
    apply_relay_mesh_env(&mut env, Some(RELAY_MESH_PROVIDER_ID), Some(model));
    env
}

#[cfg(all(test, feature = "mesh-llm"))]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn native_provider_uses_context_safe_tool_calling_budget() {
        let mut env = BTreeMap::new();
        apply_relay_mesh_env(
            &mut env,
            Some(RELAY_MESH_PROVIDER_ID),
            Some(RELAY_MESH_AUTO_MODEL_ID),
        );

        assert_eq!(
            env.get("BUZZ_AGENT_MAX_OUTPUT_TOKENS").map(String::as_str),
            Some("4096")
        );
        // Must stay unset: any value we pick overrides the model's own chat
        // template default, and the right value is model-specific ("none"
        // stops gemma tool-calling; enabling thinking makes Qwen3 burn ~4x the
        // output budget).
        assert_eq!(env.get("BUZZ_AGENT_THINKING_EFFORT"), None);
    }

    /// Stored `auto` is translated here, so buzz-agent receives a plain model
    /// name and needs no knowledge of the mesh. MeshLLM decides per request
    /// whether `mesh` becomes a committee or a single served model.
    #[test]
    fn stored_auto_becomes_the_virtual_mesh_model_on_the_wire() {
        let mut env = BTreeMap::new();
        apply_relay_mesh_env(
            &mut env,
            Some(RELAY_MESH_PROVIDER_ID),
            Some(RELAY_MESH_AUTO_MODEL_ID),
        );

        assert_eq!(
            env.get("BUZZ_AGENT_MODEL").map(String::as_str),
            Some(RELAY_MESH_VIRTUAL_MODEL_ID)
        );
        assert_eq!(
            env.get("OPENAI_COMPAT_MODEL").map(String::as_str),
            Some(RELAY_MESH_VIRTUAL_MODEL_ID)
        );
    }

    /// A blank stored model is the legacy encoding of the same intent.
    #[test]
    fn blank_stored_model_becomes_the_virtual_mesh_model() {
        let mut env = BTreeMap::new();
        apply_relay_mesh_env(&mut env, Some(RELAY_MESH_PROVIDER_ID), Some("  "));

        assert_eq!(
            env.get("BUZZ_AGENT_MODEL").map(String::as_str),
            Some(RELAY_MESH_VIRTUAL_MODEL_ID)
        );
    }

    /// Every consumer that names a model to the mesh goes through one helper,
    /// so the LLM transport and the ACP harness cannot be told different things.
    #[test]
    fn wire_model_maps_auto_and_blank_but_passes_named_through() {
        assert_eq!(
            relay_mesh_wire_model(RELAY_MESH_AUTO_MODEL_ID),
            RELAY_MESH_VIRTUAL_MODEL_ID
        );
        assert_eq!(relay_mesh_wire_model(""), RELAY_MESH_VIRTUAL_MODEL_ID);
        assert_eq!(relay_mesh_wire_model("  "), RELAY_MESH_VIRTUAL_MODEL_ID);
        assert_eq!(
            relay_mesh_wire_model("unsloth/gemma-4-E4B-it-GGUF:Q4_K_M"),
            "unsloth/gemma-4-E4B-it-GGUF:Q4_K_M"
        );
    }

    /// A named model is sent verbatim: picking one is an explicit choice to
    /// bypass mesh routing, and must not be rewritten.
    #[test]
    fn a_named_model_is_sent_verbatim() {
        let mut env = BTreeMap::new();
        apply_relay_mesh_env(
            &mut env,
            Some(RELAY_MESH_PROVIDER_ID),
            Some("unsloth/Qwen3-8B-GGUF:Q4_K_M"),
        );

        assert_eq!(
            env.get("BUZZ_AGENT_MODEL").map(String::as_str),
            Some("unsloth/Qwen3-8B-GGUF:Q4_K_M")
        );
        assert_eq!(
            env.get("OPENAI_COMPAT_MODEL").map(String::as_str),
            Some("unsloth/Qwen3-8B-GGUF:Q4_K_M")
        );
    }

    #[test]
    fn native_provider_preserves_explicit_generation_controls() {
        let mut env = BTreeMap::from([
            (
                "BUZZ_AGENT_MAX_OUTPUT_TOKENS".to_string(),
                "2048".to_string(),
            ),
            ("BUZZ_AGENT_THINKING_EFFORT".to_string(), "high".to_string()),
        ]);
        apply_relay_mesh_env(
            &mut env,
            Some(RELAY_MESH_PROVIDER_ID),
            Some(RELAY_MESH_AUTO_MODEL_ID),
        );

        assert_eq!(
            env.get("BUZZ_AGENT_MAX_OUTPUT_TOKENS").map(String::as_str),
            Some("2048")
        );
        assert_eq!(
            env.get("BUZZ_AGENT_THINKING_EFFORT").map(String::as_str),
            Some("high")
        );
    }

    #[test]
    fn native_provider_enables_reply_guard_by_default() {
        let mut env = BTreeMap::new();
        apply_relay_mesh_env(
            &mut env,
            Some(RELAY_MESH_PROVIDER_ID),
            Some(RELAY_MESH_AUTO_MODEL_ID),
        );

        assert_eq!(
            env.get("BUZZ_AGENT_REQUIRE_REPLY").map(String::as_str),
            Some("1"),
            "mesh agents opt into the reply guard automatically"
        );
    }

    #[test]
    fn native_provider_preserves_explicit_reply_guard_opt_out() {
        let mut env = BTreeMap::from([("BUZZ_AGENT_REQUIRE_REPLY".to_string(), "0".to_string())]);
        apply_relay_mesh_env(
            &mut env,
            Some(RELAY_MESH_PROVIDER_ID),
            Some(RELAY_MESH_AUTO_MODEL_ID),
        );

        assert_eq!(
            env.get("BUZZ_AGENT_REQUIRE_REPLY").map(String::as_str),
            Some("0"),
            "an explicit opt-out is a user decision, not a value to re-default"
        );
    }

    #[test]
    fn non_mesh_provider_leaves_reply_guard_unset() {
        let mut env = BTreeMap::new();
        apply_relay_mesh_env(&mut env, Some("anthropic"), Some("claude-haiku-4.5"));

        assert_eq!(
            env.get("BUZZ_AGENT_REQUIRE_REPLY"),
            None,
            "the guard stays opt-in everywhere except mesh"
        );
        assert!(env.is_empty(), "non-mesh providers get no mesh env at all");
    }

    /// The spawn path writes this map onto the command *after* the layered user
    /// env, so an explicit opt-out only survives if it is copied forward. Without
    /// the copy-forward, `apply_relay_mesh_env` re-defaults it to `1` here and
    /// silently overrides the user at spawn while readiness still shows `0`.
    #[test]
    fn process_env_preserves_explicit_reply_guard_opt_out() {
        let effective_env =
            BTreeMap::from([("BUZZ_AGENT_REQUIRE_REPLY".to_string(), "0".to_string())]);

        let env = relay_mesh_process_env(&effective_env, "Gemma-4");

        assert_eq!(
            env.get("BUZZ_AGENT_REQUIRE_REPLY").map(String::as_str),
            Some("0")
        );
    }

    #[test]
    fn process_env_enables_reply_guard_when_user_is_silent() {
        let env = relay_mesh_process_env(&BTreeMap::new(), "Gemma-4");

        assert_eq!(
            env.get("BUZZ_AGENT_REQUIRE_REPLY").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn process_env_seeds_controls_without_restoring_unrelated_credentials() {
        let effective_env = BTreeMap::from([
            (
                "BUZZ_AGENT_MAX_OUTPUT_TOKENS".to_string(),
                "1024".to_string(),
            ),
            ("OPENAI_API_KEY".to_string(), "must-not-leak".to_string()),
        ]);

        let env = relay_mesh_process_env(&effective_env, "Gemma-4");

        assert_eq!(
            env.get("BUZZ_AGENT_MAX_OUTPUT_TOKENS").map(String::as_str),
            Some("1024")
        );
        assert_eq!(
            env.get("OPENAI_COMPAT_MODEL").map(String::as_str),
            Some("Gemma-4")
        );
        assert!(!env.contains_key("OPENAI_API_KEY"));
    }
}
