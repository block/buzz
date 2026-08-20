import type { FibreKind } from "@/features/triage/api";

export const FIBRE_KIND_META: Record<
  FibreKind,
  { label: string; color: string; tint: string }
> = {
  blocker: {
    label: "Blocker",
    color: "#E88170",
    tint: "rgba(224,110,92,0.15)",
  },
  decision: {
    label: "Decision",
    color: "#A79AE8",
    tint: "rgba(151,120,255,0.15)",
  },
  ask: { label: "Ask", color: "#E5B92F", tint: "rgba(229,185,47,0.15)" },
  commitment: {
    label: "Commitment",
    color: "#E0A87A",
    tint: "rgba(224,168,122,0.15)",
  },
  idea: { label: "Idea", color: "#5FBE94", tint: "rgba(71,152,115,0.17)" },
  question: {
    label: "Question",
    color: "#7DB2F5",
    tint: "rgba(80,160,255,0.15)",
  },
  fyi: { label: "FYI", color: "#A6A6A2", tint: "rgba(255,255,255,0.08)" },
};

export function fibreKindMeta(kind: string) {
  return (
    FIBRE_KIND_META[kind as FibreKind] ?? {
      label: kind,
      color: "#A6A6A2",
      tint: "rgba(255,255,255,0.08)",
    }
  );
}
