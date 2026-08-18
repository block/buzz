export const NXTLINQ_GATEWAY_COMMAND = "nxtlinq-authorization-gateway";

const FIXED_PASS_ENV = [
  "BUZZ_AGENT_PROVIDER",
  "BUZZ_AGENT_MODEL",
  "BUZZ_AGENT_SYSTEM_PROMPT",
  "BUZZ_AGENT_SYSTEM_PROMPT_FILE",
  "BUZZ_AGENT_THINKING_EFFORT",
  "BUZZ_AGENT_THINKING_SUMMARY",
  "BUZZ_AGENT_MAX_ROUNDS",
  "BUZZ_AGENT_MAX_OUTPUT_TOKENS",
  "BUZZ_AGENT_LLM_TIMEOUT_SECS",
  "BUZZ_AGENT_TOOL_TIMEOUT_SECS",
  "BUZZ_AGENT_MCP_RESTART_MAX_ATTEMPTS",
  "BUZZ_AGENT_MCP_RESTART_BASE_MS",
  "BUZZ_AGENT_MCP_RESTART_MAX_MS",
  "BUZZ_AGENT_MAX_SESSIONS",
  "BUZZ_AGENT_MAX_LINE_BYTES",
  "BUZZ_AGENT_MAX_HISTORY_BYTES",
  "BUZZ_AGENT_MAX_CONTEXT_TOKENS",
  "BUZZ_AGENT_MAX_HANDOFFS",
  "BUZZ_AGENT_MAX_PARALLEL_TOOLS",
  "BUZZ_AGENT_HOOK_TIMEOUT_MS",
  "BUZZ_AGENT_STOP_MAX_REJECTIONS",
  "BUZZ_AGENT_NXTLINQ_PERMISSION_BRIDGE",
  "BUZZ_AGENT_REQUIRE_REPLY",
  "BUZZ_AGENT_NO_HINTS",
  "BUZZ_AGENT_PROMPT_CACHING",
  "ANTHROPIC_API_KEY",
  "ANTHROPIC_MODEL",
  "ANTHROPIC_BASE_URL",
  "ANTHROPIC_API_VERSION",
  "OPENAI_COMPAT_API_KEY",
  "OPENAI_COMPAT_MODEL",
  "OPENAI_COMPAT_BASE_URL",
  "OPENAI_COMPAT_API",
  "OPENROUTER_API_KEY",
  "OPENROUTER_MODEL",
  "OPENROUTER_BASE_URL",
  "DATABRICKS_HOST",
  "DATABRICKS_MODEL",
  "DATABRICKS_TOKEN",
  "MCP_HOOK_SERVERS",
] as const;

export type NxtlinqLaunchPreset = {
  project: string;
  trustStore: string;
  receiptDirectory: string;
};

export function nxtlinqLaunchPresetMatches(
  left: NxtlinqLaunchPreset | null,
  right: NxtlinqLaunchPreset,
): boolean {
  return (
    left !== null &&
    left.project.trim() === right.project.trim() &&
    left.trustStore.trim() === right.trustStore.trim() &&
    left.receiptDirectory.trim() === right.receiptDirectory.trim()
  );
}

export function shouldBlockNxtlinqLaunchSave(input: {
  enabled: boolean;
  appliedPreset: NxtlinqLaunchPreset | null;
  draftPreset: NxtlinqLaunchPreset;
  draftVerified: boolean;
}): boolean {
  return (
    input.enabled &&
    !nxtlinqLaunchPresetMatches(input.appliedPreset, input.draftPreset) &&
    !input.draftVerified
  );
}

export function isNxtlinqGatewayCommand(command: string): boolean {
  const basename = command.trim().split(/[\\/]/).at(-1) ?? "";
  return basename === NXTLINQ_GATEWAY_COMMAND;
}

export function parseNxtlinqLaunchPreset(
  args: readonly string[],
): NxtlinqLaunchPreset | null {
  function valueAfter(flag: string): string {
    const index = args.indexOf(flag);
    return index >= 0 ? (args[index + 1] ?? "") : "";
  }
  const project = valueAfter("--project");
  const trustStore = valueAfter("--trust-store");
  const receiptDirectory = valueAfter("--receipt-dir");
  return project || trustStore || receiptDirectory
    ? { project, trustStore, receiptDirectory }
    : null;
}

export function deriveNxtlinqOperatorDefaults(workspace: string): {
  trustStore: string;
  receiptDirectory: string;
} {
  const normalized = workspace.trim().replace(/[\\/]+$/, "");
  const separator = normalized.includes("\\") ? "\\" : "/";
  const index = Math.max(
    normalized.lastIndexOf("/"),
    normalized.lastIndexOf("\\"),
  );
  const parent = index >= 0 ? normalized.slice(0, index) : "";
  const name = index >= 0 ? normalized.slice(index + 1) : normalized;
  const operatorDirectory = `${parent}${parent ? separator : ""}.${name}-operator`;
  return {
    trustStore: `${operatorDirectory}${separator}trusted-signers.json`,
    receiptDirectory: `${operatorDirectory}${separator}receipts`,
  };
}

export function deriveNxtlinqReceiptDirectory(
  receiptRoot: string,
  agentPubkey: string,
): string {
  const root = receiptRoot.trim().replace(/[\\/]+$/, "");
  const separator = root.includes("\\") ? "\\" : "/";
  const safeAgentId = agentPubkey.trim().replace(/[^A-Za-z0-9_-]/g, "-");
  return root && safeAgentId ? `${root}${separator}${safeAgentId}` : "";
}

export function buildNxtlinqWrapperArgs(input: {
  project: string;
  trustStore: string;
  receiptDirectory: string;
  passEnvironment: readonly string[];
}): string[] {
  const values = [input.project, input.trustStore, input.receiptDirectory];
  if (values.some((value) => value.includes(","))) {
    throw new Error(
      "Nxtlinq paths cannot contain commas in the current Buzz argv transport.",
    );
  }
  const passEnvironment = Array.from(
    new Set([...FIXED_PASS_ENV, ...input.passEnvironment]),
  )
    .filter((name) => /^[A-Za-z_][A-Za-z0-9_]*$/.test(name))
    .filter((name) => name !== "BUZZ_ACP_TRUST_NXTLINQ_GATEWAY")
    .sort();
  return [
    "--adapter",
    "acp",
    "--project",
    input.project,
    "--trust-store",
    input.trustStore,
    "--receipt-dir",
    input.receiptDirectory,
    "--mode",
    "acp-enforce",
    ...passEnvironment.flatMap((name) => ["--pass-env", name]),
    "--forward-agent-stderr",
    "--",
  ];
}
