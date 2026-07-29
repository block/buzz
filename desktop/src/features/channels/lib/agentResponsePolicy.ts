import type { AgentResponsePolicy } from "@/shared/api/types";

export function agentResponseEmoji(policy: AgentResponsePolicy): string {
  return policy === "all" ? "💬" : "🏷️";
}

export function agentResponseLabel(policy: AgentResponsePolicy): string {
  return policy === "all" ? "Every message" : "Only @mentions";
}
