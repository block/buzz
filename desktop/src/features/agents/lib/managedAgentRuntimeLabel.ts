import type { ManagedAgent } from "@/shared/api/types";

const RUNTIME_LABELS: Record<string, string> = {
  goose: "Goose",
  "claude-code": "Claude Code",
  "codex-acp": "Codex",
  hermes: "Hermes",
  aider: "Aider",
};

export function runtimeLabel(command: string | null | undefined): string {
  const value = command?.trim() ?? "";
  return RUNTIME_LABELS[value.toLowerCase()] ?? (value || "Custom");
}

/**
 * Provider ownership is authoritative for display. A stale or legacy harness
 * command must not make a native provider agent appear to be Codex/ACP.
 */
export function managedAgentRuntimeLabel(
  agent: Pick<ManagedAgent, "agentCommand" | "backend">,
): string {
  if (agent.backend.type === "provider") {
    const providerId = agent.backend.id.trim().toLowerCase();
    if (providerId === "hermes") return "Hermes";
    return providerId ? `Remote (${providerId})` : "Remote";
  }
  return runtimeLabel(agent.agentCommand);
}
