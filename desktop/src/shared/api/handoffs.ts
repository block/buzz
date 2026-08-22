import { invokeTauri } from "./tauri";

export type AgentHandoffSummary = {
  eventId: string;
  senderPubkey: string;
  createdAt: number;
  title: string;
  summary: string | null;
};

export type AgentHandoffRecord = AgentHandoffSummary & {
  history: string;
};

export function sendAgentHandoff(input: {
  recipientPubkey: string;
  title: string;
  summary?: string;
  history: string;
}) {
  return invokeTauri<string>("send_agent_handoff", { request: input });
}

export function listAgentHandoffs(limit = 50) {
  return invokeTauri<AgentHandoffSummary[]>("list_agent_handoffs", { limit });
}

export function getAgentHandoff(eventId: string) {
  return invokeTauri<AgentHandoffRecord>("get_agent_handoff", { eventId });
}
