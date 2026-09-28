/** Threads Assistant has joined — tracked for Summarize + history. */

import type { BestieListScope } from "./bestieListTypes";

export type BestieThreadScope = BestieListScope;

/** How the thread entered local tracking. */
export type BestieThreadSource = "ask" | "agent" | "add";

export type BestieTrackedThread = {
  addedAt: number;
  authorName: string | null;
  channelId: string;
  channelName: string | null;
  /** Composite id: `${channelId}:${rootEventId}` */
  id: string;
  lastActiveAt: number;
  lastSummary: string | null;
  lastSummaryAt: number | null;
  /** Short preview from the message that enrolled Assistant. */
  preview: string;
  rootEventId: string;
  /** ask = Ask Assistant; agent = agent authored in thread; add = Threads +. */
  source: BestieThreadSource;
};

export type BestieThreadPendingSummarize = {
  startedAt: number;
  threadId: string;
  /** Owner summarize prompt message id once posted (for in-thread capture). */
  triggerMessageId: string | null;
};

export type BestieThreadState = {
  pendingSummarize: BestieThreadPendingSummarize | null;
  threads: BestieTrackedThread[];
  version: 1;
};

export type BestieThreadUpsertInput = {
  authorName?: string | null;
  channelId: string;
  channelName?: string | null;
  /** Optional activity timestamp (unix seconds). Bumps lastActiveAt when newer. */
  lastActiveAt?: number;
  preview: string;
  rootEventId: string;
  source?: BestieThreadSource;
};
