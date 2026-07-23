import type { AcpRuntime, GlobalAgentConfig } from "@/shared/api/types";

/** Sentinel preferred_runtime / selector id for a bring-your-own ACP command. */
export const CUSTOM_RUNTIME_ID = "custom";

export const CUSTOM_RUNTIME_LABEL = "Custom command";

export type ByoHarnessDraft = {
  args: string;
  command: string;
};

/**
 * Build a synthetic runtime for a user-supplied ACP command.
 *
 * Custom harnesses are outside `KNOWN_ACP_RUNTIMES`. `availability: "available"`
 * means the user configured a command — not that PATH was probed. Spawn resolves
 * the binary later the same way it does for any `agent_command` pin.
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
    // Mirror command: spawn uses the command string; absolute paths work as-is,
    // bare names are resolved on PATH at spawn time.
    binaryPath: trimmed,
    defaultArgs: args.map((arg) => arg.trim()).filter((arg) => arg.length > 0),
    mcpCommand: null,
    modelEnvVar: null,
    providerEnvVar: null,
    thinkingEnvVar: null,
    installHint:
      "Any ACP-over-stdio binary. Buzz does not verify PATH until the agent starts.",
    installInstructionsUrl: "https://agentclientprotocol.com/",
    canAutoInstall: false,
    underlyingCliPath: null,
    nodeRequired: false,
    authStatus: { status: "not_applicable" },
    loginHint: null,
  };
}

/** Placeholder catalog entry while the custom command field is still empty. */
export function customHarnessCatalogStub(): AcpRuntime {
  return {
    id: CUSTOM_RUNTIME_ID,
    label: CUSTOM_RUNTIME_LABEL,
    avatarUrl: "",
    availability: "available",
    command: "",
    binaryPath: "",
    defaultArgs: [],
    mcpCommand: null,
    modelEnvVar: null,
    providerEnvVar: null,
    thinkingEnvVar: null,
    installHint: "",
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

export function formatAgentArgsInput(
  args: readonly string[] | null | undefined,
) {
  return (args ?? []).join(", ");
}

export function applyCustomHarnessPreference(
  config: GlobalAgentConfig,
  draft: ByoHarnessDraft,
): GlobalAgentConfig {
  return {
    ...config,
    preferred_runtime: CUSTOM_RUNTIME_ID,
    preferred_agent_command: draft.command.trim(),
    preferred_agent_args: parseAgentArgsInput(draft.args),
    // Custom harnesses don't use Buzz provider/model knobs.
    model: null,
    provider: null,
  };
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
