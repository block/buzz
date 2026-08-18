//! OpenCode discovery tests — split out of `tests.rs` to keep it under the
//! repo's file-size ratchet.

use super::normalize_agent_args;

/// OpenCode was promoted from a preset to a builtin runtime so it could carry
/// a `config_file_path` — its model lives only in its config file. Two things
/// had to survive that move, and both are silent if they break: the runtime
/// must resolve by command (or the config bridge sees no metadata at all), and
/// it must still spawn as `opencode acp` (builtins get their args from
/// `default_agent_args`, not from the preset's `args` list, so an omission here
/// would launch the bare CLI instead of the ACP server).
#[test]
fn opencode_resolves_as_a_builtin_and_keeps_its_acp_arg() {
    let runtime = super::known_acp_runtime("opencode").expect("opencode should be a known runtime");
    assert_eq!(runtime.id, "opencode");
    assert_eq!(
        runtime.config_file_path,
        Some("~/.config/opencode/opencode.json")
    );
    assert!(
        runtime.model_env_var.is_none(),
        "opencode has no model env var — that is why it needs the config file"
    );

    assert_eq!(
        normalize_agent_args("opencode", Vec::new()),
        vec!["acp".to_string()]
    );
    assert_eq!(
        normalize_agent_args("opencode", vec!["acp".into()]),
        vec!["acp".to_string()]
    );
}
