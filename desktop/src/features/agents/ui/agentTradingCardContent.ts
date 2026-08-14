import type {
  SnapshotFormat,
  SnapshotMemoryLevel,
} from "@/shared/api/tauriPersonas";

export const AGENT_CARD_WIDTH = 1227;
export const AGENT_CARD_HEIGHT = 1839;

export const AGENT_CARD_MEMORY_LABELS: Record<SnapshotMemoryLevel, string> = {
  none: "Agent only",
  core: "Core memory",
  everything: "All memories",
};

export function truncateAgentCardTitle(agentName: string): string {
  const title = agentName.trim().toLocaleUpperCase() || "BUZZ AGENT";
  const characters = Array.from(title);
  return characters.length > 26
    ? `${characters.slice(0, 25).join("")}…`
    : title;
}

export function agentCardTitleSize(title: string): number {
  if (title.length <= 16) return 68;
  if (title.length <= 21) return 58;
  return 49;
}

export function stableAgentCardNumber(agentId: string): string {
  let hash = 0;
  for (const character of agentId) {
    hash = (hash * 31 + (character.codePointAt(0) ?? 0)) >>> 0;
  }
  return String(hash % 10_000).padStart(4, "0");
}

export function agentCardFormatLabel(format: SnapshotFormat): string {
  return format.toUpperCase();
}
