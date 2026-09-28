/** Bestie Jobs — autonomous scheduled prompts (separate from reminders/todos). */

import type { BestieListScope } from "./bestieListTypes";

export type BestieJobScope = BestieListScope;

/** One-shot or recurring schedule for a Bestie job. */
export type BestieJobSchedule =
  | { dueAt: number; kind: "once" }
  | { everySeconds: number; kind: "interval" }
  | { hour: number; kind: "daily"; minute: number }
  | { hour: number; kind: "weekly"; minute: number; weekday: number };

export type BestieJob = {
  createdAt: number;
  enabled: boolean;
  id: string;
  /** Unix seconds of last successful fire, or null. */
  lastRunAt: number | null;
  /** Unix seconds when this job should next fire, or null when disabled/done. */
  nextDueAt: number | null;
  /** Instructions Bestie runs when the job fires. */
  prompt: string;
  schedule: BestieJobSchedule;
  sourceMessageId: string | null;
  title: string;
  updatedAt: number;
};

export type BestieJobState = {
  /** Slots already fired: `${jobId}@${dueAt}` — one fire per due slot. */
  firedSlotIds: string[];
  jobs: BestieJob[];
  processedMessageIds: string[];
  version: 1;
};

export type BestieJobAddInput = {
  enabled?: boolean;
  prompt: string;
  schedule: BestieJobSchedule;
  sourceMessageId?: string | null;
  title: string;
};

export type BestieJobUpdateInput = {
  enabled?: boolean;
  id: string;
  prompt?: string;
  schedule?: BestieJobSchedule;
  title?: string;
};
