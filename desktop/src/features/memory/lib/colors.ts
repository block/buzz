import type { NodeEntityType } from "./types";

export interface NodeVisualMeta {
  color: string;
  glowColor: string;
  badgeBg: string;
  badgeText: string;
  badgeBorder: string;
  defaultRadius: number;
  label: string;
}

export const OBSIDIAN_NODE_PALETTE: Record<string, NodeVisualMeta> = {
  project: {
    color: "#10B981", // Emerald
    glowColor: "rgba(16, 185, 129, 0.4)",
    badgeBg: "rgba(16, 185, 129, 0.15)",
    badgeText: "#34D399",
    badgeBorder: "rgba(16, 185, 129, 0.35)",
    defaultRadius: 13,
    label: "Project",
  },
  chat: {
    color: "#06B6D4", // Cyan
    glowColor: "rgba(6, 182, 212, 0.4)",
    badgeBg: "rgba(6, 182, 212, 0.15)",
    badgeText: "#22D3EE",
    badgeBorder: "rgba(6, 182, 212, 0.35)",
    defaultRadius: 10,
    label: "Chat Thread",
  },
  agent: {
    color: "#8B5CF6", // Violet
    glowColor: "rgba(139, 92, 246, 0.45)",
    badgeBg: "rgba(139, 92, 246, 0.15)",
    badgeText: "#A78BFA",
    badgeBorder: "rgba(139, 92, 246, 0.35)",
    defaultRadius: 11,
    label: "Agent",
  },
  file: {
    color: "#F59E0B", // Amber
    glowColor: "rgba(245, 158, 11, 0.35)",
    badgeBg: "rgba(245, 158, 11, 0.15)",
    badgeText: "#FBBF24",
    badgeBorder: "rgba(245, 158, 11, 0.35)",
    defaultRadius: 8,
    label: "File / Doc",
  },
  concept: {
    color: "#EAB308", // Yellow Gold
    glowColor: "rgba(234, 179, 8, 0.35)",
    badgeBg: "rgba(234, 179, 8, 0.15)",
    badgeText: "#FDE047",
    badgeBorder: "rgba(234, 179, 8, 0.35)",
    defaultRadius: 7,
    label: "Concept",
  },
  decision: {
    color: "#F8FAFC", // Slate White
    glowColor: "rgba(248, 250, 252, 0.5)",
    badgeBg: "rgba(248, 250, 252, 0.12)",
    badgeText: "#F8FAFC",
    badgeBorder: "rgba(248, 250, 252, 0.3)",
    defaultRadius: 8,
    label: "Decision",
  },
  technology: {
    color: "#6366F1", // Indigo
    glowColor: "rgba(99, 102, 241, 0.4)",
    badgeBg: "rgba(99, 102, 241, 0.15)",
    badgeText: "#818CF8",
    badgeBorder: "rgba(99, 102, 241, 0.35)",
    defaultRadius: 8,
    label: "Technology",
  },
  symbol: {
    color: "#38BDF8", // Sky
    glowColor: "rgba(56, 189, 248, 0.35)",
    badgeBg: "rgba(56, 189, 248, 0.15)",
    badgeText: "#7DD3FC",
    badgeBorder: "rgba(56, 189, 248, 0.35)",
    defaultRadius: 7,
    label: "Symbol",
  },
  default: {
    color: "#94A3B8", // Slate
    glowColor: "rgba(148, 163, 184, 0.25)",
    badgeBg: "rgba(148, 163, 184, 0.15)",
    badgeText: "#CBD5E1",
    badgeBorder: "rgba(148, 163, 184, 0.3)",
    defaultRadius: 7,
    label: "Entity",
  },
};

export function getNodeVisualMeta(entityType?: NodeEntityType | string): NodeVisualMeta {
  if (!entityType) return OBSIDIAN_NODE_PALETTE.default;
  const key = entityType.toLowerCase();
  return OBSIDIAN_NODE_PALETTE[key] ?? OBSIDIAN_NODE_PALETTE.default;
}

export const EDGE_COLORS = {
  active: "rgba(148, 163, 184, 0.28)",
  highlighted: "rgba(255, 255, 255, 0.85)",
  supersedes: "rgba(239, 68, 68, 0.65)", // Dashed red/amber
  label: "rgba(148, 163, 184, 0.65)",
};
