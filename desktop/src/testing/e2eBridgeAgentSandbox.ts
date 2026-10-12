/**
 * Agent sandbox commands for the e2e bridge, plus the bridge's
 * unsupported-command fallback.
 *
 * Kept outside e2eBridge.ts, which is over the file-size limit and may not
 * grow: the bridge delegates every command it does not handle here.
 */

const MOCK_PROFILE_PATH =
  "/Users/mock/.config/nono/profiles/buzz-agent-local.json";

let mockSandboxEnabled = true;

export function handleAgentSandboxMockCommand(
  command: string,
  payload: unknown,
): unknown {
  switch (command) {
    case "get_agent_sandbox_status":
      return {
        supported: true,
        enabled: mockSandboxEnabled,
        problem: null,
        profile_path: MOCK_PROFILE_PATH,
      };
    case "set_agent_sandbox_enabled":
      mockSandboxEnabled = (payload as { enabled: boolean }).enabled;
      // E2E runs no local agents, so nothing restarts.
      return {
        config: {
          env_vars: {},
          provider: null,
          model: null,
          preferred_runtime: null,
          disable_agent_sandbox: !mockSandboxEnabled,
        },
        restarted_count: 0,
        failed_restart_count: 0,
      };
    case "open_agent_sandbox_profile":
    case "reset_agent_sandbox_profile":
      return null;
    default:
      throw new Error(`Unsupported mocked Tauri command: ${command}`);
  }
}
