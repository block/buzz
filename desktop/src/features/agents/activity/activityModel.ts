// Core data model for the agent activity visualization.
//
// Ported from Berd's agent-activity-panel feature (branch
// zmarley/agent-activity-panel) and adapted for Buzz: source references
// point at observer-frame identifiers (seq / toolCallId / turnId) instead
// of Berd chat-message indices.

/**
 * Activity event kinds: r=read, w=write, c=create, s=status (pathless),
 * sp=spawn, d=done, x=contention. Derivation emits only r|w|c|s; sp/d come
 * from agent lifecycle and x is synthesized by the scene compiler.
 */
export type ActivityEventKind = "r" | "w" | "c" | "s" | "sp" | "d" | "x";

export interface ActivityEvent {
  agentId: string;
  /** Epoch milliseconds. */
  t: number;
  kind: ActivityEventKind;
  path?: string;
  label?: string;
  /** Agent narration immediately preceding this tool call, if any. */
  intent?: string;
  /** Observer-frame sequence number of the originating acp_read frame. */
  sourceSeq?: number;
  sourceToolCallId?: string;
  sourceTurnId?: string;
}

export interface ActivityAgent {
  id: string;
  name: string;
  color: string;
  spawnT: number;
  doneT?: number;
}

/**
 * Distinguishable-on-dark agent palette. Index-stable so a channel's agents
 * keep their colors across recompiles.
 */
const AGENT_COLOR_PALETTE: readonly string[] = [
  "#7bd88f", // green — Berd's original single-agent color
  "#6fb7ff",
  "#f5a97f",
  "#c6a0f6",
  "#eed49f",
  "#8bd5ca",
  "#f28fad",
  "#a6da95",
];

/** Stable palette pick: by explicit index when known, else FNV-1a of the id. */
export function colorForAgent(agentId: string, index?: number): string {
  if (index !== undefined && index >= 0) {
    return AGENT_COLOR_PALETTE[index % AGENT_COLOR_PALETTE.length];
  }
  let hash = 0x811c9dc5;
  for (let i = 0; i < agentId.length; i++) {
    hash ^= agentId.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return AGENT_COLOR_PALETTE[(hash >>> 0) % AGENT_COLOR_PALETTE.length];
}
