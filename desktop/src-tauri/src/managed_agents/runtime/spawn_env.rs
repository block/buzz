use std::path::Path;
use std::process::Command;

use crate::managed_agents::{
    known_acp_runtime, requires_durable_thread_sessions, ManagedAgentRecord,
};

pub(super) struct HarnessProcessEnv<'a> {
    pub record: &'a ManagedAgentRecord,
    pub relay_url: &'a str,
    pub lazy: bool,
    pub effective_command: &'a str,
    pub resolved_agent_command: &'a str,
    pub agent_args: &'a [String],
    pub resolved_mcp_command: Option<&'a Path>,
}

/// Apply the Buzz-owned harness environment as one unit. Keeping these values
/// together makes the thread-session safety flag inseparable from the adapter
/// command it describes and keeps the process-spawn boundary reviewable.
pub(super) fn apply_harness_process_env(command: &mut Command, env: HarnessProcessEnv<'_>) {
    command.env("RUST_LOG", super::child_rust_log_filter());
    command.env("BUZZ_PRIVATE_KEY", &env.record.private_key_nsec);
    command.env("BUZZ_RELAY_URL", env.relay_url);
    command.env("BUZZ_ACP_LAZY_POOL", env.lazy.to_string());
    command.env("BUZZ_ACP_AGENT_COMMAND", env.resolved_agent_command);
    command.env("BUZZ_ACP_AGENT_ARGS", env.agent_args.join(","));
    command.env(
        "BUZZ_ACP_REQUIRE_DURABLE_THREAD_SESSIONS",
        requires_durable_thread_sessions(env.effective_command).to_string(),
    );
    command.env(
        "BUZZ_ACP_MCP_COMMAND",
        env.resolved_mcp_command
            .map_or_else(|| "".into(), Path::to_path_buf),
    );
    if known_acp_runtime(env.effective_command).is_some_and(|runtime| runtime.mcp_hooks) {
        // `build_mcp_servers()` fixes the server name to `buzz-mcp`.
        command.env("MCP_HOOK_SERVERS", "*");
    }
}
