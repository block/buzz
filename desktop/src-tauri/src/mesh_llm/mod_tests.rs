//! Unit tests for `mesh_llm/mod.rs` private helpers (kept in a sibling file so
//! `mod.rs` stays under the 500-line budget; `#[path]`-included from there).
use super::{find_progressish_reason, looks_like_model_ref};
use serde_json::json;

#[test]
fn progressish_reads_typed_phase_not_whole_tree() {
    assert_eq!(
        find_progressish_reason(&json!({"phase": "downloading weights"})),
        Some("downloading model".to_string())
    );
    // Regression (Sami N1): an unrelated field mentioning a progress word must
    // not trip the badge — only the typed phase field counts.
    assert_eq!(
        find_progressish_reason(&json!({
            "phase": "ready",
            "model_name": "prepared-qwen-preparing"
        })),
        None
    );
    assert_eq!(find_progressish_reason(&json!({"foo": "bar"})), None);
}

#[test]
fn model_ref_is_family_agnostic() {
    assert!(looks_like_model_ref("hf://org/model"));
    assert!(looks_like_model_ref("some-model.gguf"));
    assert!(looks_like_model_ref("Some-Model.GGUF"));
    // Families that used to be hardcoded must route via the structured path,
    // not a name allowlist here (Sami N2):
    assert!(!looks_like_model_ref("Mistral-7B"));
    assert!(!looks_like_model_ref("Qwen3-35B"));
    assert!(!looks_like_model_ref(""));
}

#[test]
fn agent_preset_runs_on_sprout_agent_not_goose() {
    // Regression (Tyler): the relay-mesh preset used to hand the agent the
    // global default runtime (goose), which ignores the OpenAI-compat env
    // vars and falls back to its own provider. Mesh agents must run on
    // sprout-agent, which reads those vars.
    let preset = super::agent_preset(super::MeshAgentPresetRequest {
        model_id: "Qwen3-8B-Q4_K_M".to_string(),
    })
    .expect("preset for a valid model id");

    assert_eq!(preset.agent_command, "sprout-agent");
    assert_ne!(preset.agent_command, "goose");
    assert_eq!(preset.mcp_command, "sprout-dev-mcp");

    // The env vars sprout-agent's config layer reads (crates/sprout-agent).
    assert_eq!(
        preset
            .env_vars
            .get("SPROUT_AGENT_PROVIDER")
            .map(String::as_str),
        Some("openai")
    );
    assert_eq!(
        preset
            .env_vars
            .get("OPENAI_COMPAT_MODEL")
            .map(String::as_str),
        Some("Qwen3-8B-Q4_K_M")
    );
}
