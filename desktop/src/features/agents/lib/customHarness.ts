import type { AcpRuntime, GlobalAgentConfig } from "@/shared/api/types";

/** Sentinel preferred_runtime / selector id for a bring-your-own ACP command. */
export const CUSTOM_RUNTIME_ID = "custom";

export const CUSTOM_RUNTIME_LABEL = "Custom command";

/**
 * Build a synthetic available runtime for a user-supplied ACP command.
 *
 * Custom harnesses are outside the Rust `KNOWN_ACP_RUNTIMES` catalog — they
 * have no install/auth/model capability metadata. Spawn still works because
 * buzz-acp accepts any stdio ACP server via `BUZZ_ACP_AGENT_COMMAND`.
 */
export function buildCustomAcpRuntime(
  command: string,
  args: readonly string[] = [],
): AcpRuntime | null {
  const trimmed = command.trim();
  if (!trimmed) return null;

  return {
    id: CUSTOM_RUNTIME_ID,
    label: CUSTOM_RUNTIME_LABEL,
    avatarUrl: "",
    availability: "available",
    command: trimmed,
    binaryPath: trimmed,
    defaultArgs: args.map((arg) => arg.trim()).filter((arg) => arg.length > 0),
    mcpCommand: null,
    modelEnvVar: null,
    providerEnvVar: null,
    thinkingEnvVar: null,
    installHint:
      "Point Buzz at any ACP-speaking binary (Cursor `agent`, yoak, OpenCode, …).",
    installInstructionsUrl: "https://agentclientprotocol.com/",
    canAutoInstall: false,
    underlyingCliPath: null,
    nodeRequired: false,
    authStatus: { status: "not_applicable" },
    loginHint: null,
  };
}

export function isCustomRuntimeId(runtimeId: string | null | undefined) {
  return (runtimeId ?? "").trim() === CUSTOM_RUNTIME_ID;
}

export function parseAgentArgsInput(value: string): string[] {
  return value
    .split(",")
    .map((arg) => arg.trim())
    .filter((arg) => arg.length > 0);
}

export function formatAgentArgsInput(args: readonly string[] | null | undefined) {
  return (args ?? []).join(", ");
}

/** Resolve the effective preferred harness, including a BYO custom command. */
export function resolvePreferredCustomRuntime(
  config: Pick<
    GlobalAgentConfig,
    "preferred_runtime" | "preferred_agent_command" | "preferred_agent_args"
  >,
): AcpRuntime | null {
  if (!isCustomRuntimeId(config.preferred_runtime)) return null;
  return buildCustomAcpRuntime(
    config.preferred_agent_command ?? "",
    config.preferred_agent_args ?? [],
  );
}
