import type { ReactNode } from "react";

/** Prepared UI input, not an observer event or a second runtime model. */
export type WorkStepPresentation = {
  id: string;
  title: string;
  kind: "tool" | "progress" | "thought";
  status: "running" | "completed" | "failed" | "unavailable";
  detail?: string;
};

/** The activity owner supplies ordering, visibility, summaries, and actions. */
export type AgentWorkPresentation = {
  id: string;
  agent: { name: string; avatar?: string; fallback: string };
  status:
    | "running"
    | "completed"
    | "waiting"
    | "needs-you"
    | "failed"
    | "cancelled"
    | "unavailable";
  statusLabel: string;
  summary: string;
  earlierSteps: readonly WorkStepPresentation[];
  visibleSteps: readonly WorkStepPresentation[];
  notice?: { title: string; description: string; actions?: ReactNode };
  artifacts?: readonly { id: string; name: string; preview: ReactNode }[];
};
