import {
  bestieOwnerStorageKey,
  readOwnerScopedState,
  writeOwnerScopedState,
} from "./bestieOwnerScope";

import { bestieThreadId } from "./bestieThreadProtocol";
import { BESTIE_THREAD_SUMMARIZE_START_GRACE_SECONDS } from "./bestieThreadSummarizeLive";
import type {
  BestieThreadScope,
  BestieThreadState,
  BestieThreadUpsertInput,
  BestieTrackedThread,
} from "./bestieThreadTypes";

export const BESTIE_THREAD_STORAGE_PREFIX = "buzz-bestie-threads.v1";
export const BESTIE_THREAD_MAX = 80;

/** Owner+relay key — persists across Assistant agent reassignment. */
export function bestieThreadStorageKey(scope: BestieThreadScope): string {
  return bestieOwnerStorageKey(BESTIE_THREAD_STORAGE_PREFIX, scope);
}

export const EMPTY_BESTIE_THREAD_STATE: BestieThreadState = Object.freeze({
  pendingSummarize: null,
  threads: Object.freeze([]) as unknown as BestieTrackedThread[],
  version: 1,
});

export function emptyBestieThreadState(): BestieThreadState {
  return EMPTY_BESTIE_THREAD_STATE;
}

function isFiniteNonNegative(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

function parseThread(value: unknown): BestieTrackedThread | null {
  if (typeof value !== "object" || value === null) return null;
  const record = value as Record<string, unknown>;
  if (
    typeof record.id !== "string" ||
    typeof record.channelId !== "string" ||
    typeof record.rootEventId !== "string" ||
    typeof record.preview !== "string" ||
    !isFiniteNonNegative(record.addedAt) ||
    !isFiniteNonNegative(record.lastActiveAt)
  ) {
    return null;
  }
  const source =
    record.source === "ask" ||
    record.source === "agent" ||
    record.source === "add"
      ? record.source
      : "ask";
  return {
    addedAt: Math.floor(record.addedAt),
    authorName:
      typeof record.authorName === "string" && record.authorName.trim()
        ? record.authorName.trim()
        : null,
    channelId: record.channelId,
    channelName:
      typeof record.channelName === "string" && record.channelName.trim()
        ? record.channelName.trim()
        : null,
    id: record.id,
    lastActiveAt: Math.floor(record.lastActiveAt),
    lastSummary:
      typeof record.lastSummary === "string" && record.lastSummary.trim()
        ? record.lastSummary
        : null,
    lastSummaryAt: isFiniteNonNegative(record.lastSummaryAt)
      ? Math.floor(record.lastSummaryAt)
      : null,
    preview: record.preview.trim(),
    rootEventId: record.rootEventId,
    source,
  };
}

export function parseBestieThreadState(
  value: unknown,
): BestieThreadState | null {
  if (typeof value !== "object" || value === null) return null;
  const record = value as Record<string, unknown>;
  if (record.version !== 1 || !Array.isArray(record.threads)) return null;
  const threads: BestieTrackedThread[] = [];
  for (const entry of record.threads) {
    const parsed = parseThread(entry);
    if (parsed) threads.push(parsed);
  }
  let pendingSummarize: BestieThreadState["pendingSummarize"] = null;
  if (
    typeof record.pendingSummarize === "object" &&
    record.pendingSummarize !== null
  ) {
    const pending = record.pendingSummarize as Record<string, unknown>;
    if (
      typeof pending.threadId === "string" &&
      isFiniteNonNegative(pending.startedAt)
    ) {
      const triggerMessageId =
        typeof pending.triggerMessageId === "string" &&
        pending.triggerMessageId.length > 0
          ? pending.triggerMessageId
          : null;
      pendingSummarize = clearStaleThreadPendingOnLoad({
        startedAt: Math.floor(pending.startedAt),
        threadId: pending.threadId,
        triggerMessageId,
      });
    }
  }
  return { pendingSummarize, threads, version: 1 };
}

/** Drop stuck summarize pending restored from localStorage after reload. */
export function clearStaleThreadPendingOnLoad(
  pending: BestieThreadState["pendingSummarize"],
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieThreadState["pendingSummarize"] {
  if (!pending) return null;
  const age = nowSeconds - pending.startedAt;
  if (age < 0) return null;
  if (age > BESTIE_THREAD_SUMMARIZE_START_GRACE_SECONDS) return null;
  return pending;
}

function mergeBestieThreadStates(
  into: BestieThreadState,
  from: BestieThreadState,
): BestieThreadState {
  const byId = new Map(into.threads.map((thread) => [thread.id, thread]));
  for (const thread of from.threads) {
    const existing = byId.get(thread.id);
    if (
      !existing ||
      thread.lastActiveAt >= existing.lastActiveAt ||
      (thread.lastSummaryAt ?? 0) >= (existing.lastSummaryAt ?? 0)
    ) {
      byId.set(thread.id, thread);
    }
  }
  return {
    pendingSummarize: into.pendingSummarize ?? from.pendingSummarize,
    threads: [...byId.values()],
    version: 1,
  };
}

export function readBestieThreadState(scope: BestieThreadScope): BestieThreadState {
  return readOwnerScopedState({
    empty: emptyBestieThreadState,
    isEmpty: (state) => state.threads.length === 0 && state.pendingSummarize == null,
    merge: mergeBestieThreadStates,
    parse: parseBestieThreadState,
    prefix: BESTIE_THREAD_STORAGE_PREFIX,
    scope,
  });
}

export function writeBestieThreadState(
  scope: BestieThreadScope,
  state: BestieThreadState,
): void {
  writeOwnerScopedState(BESTIE_THREAD_STORAGE_PREFIX, scope, state);
}

export function upsertBestieTrackedThread(
  state: BestieThreadState,
  input: BestieThreadUpsertInput,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieThreadState {
  const channelId = input.channelId.trim();
  const rootEventId = input.rootEventId.trim();
  if (!channelId || !rootEventId) return state;
  const id = bestieThreadId(channelId, rootEventId);
  const preview = input.preview.trim().slice(0, 280);
  const existing = state.threads.find((thread) => thread.id === id);
  const source = input.source ?? existing?.source ?? "ask";
  const activityAt =
    typeof input.lastActiveAt === "number" &&
    Number.isFinite(input.lastActiveAt) &&
    input.lastActiveAt >= 0
      ? Math.floor(input.lastActiveAt)
      : nowSeconds;
  const nextThread: BestieTrackedThread = existing
    ? {
        ...existing,
        authorName: input.authorName?.trim() || existing.authorName,
        channelName: input.channelName?.trim() || existing.channelName,
        lastActiveAt: Math.max(existing.lastActiveAt, activityAt),
        preview: preview || existing.preview,
        // Prefer explicit agent/add over legacy ask when rediscovered.
        source:
          existing.source === "ask" && source !== "ask"
            ? source
            : existing.source,
      }
    : {
        addedAt: nowSeconds,
        authorName: input.authorName?.trim() || null,
        channelId,
        channelName: input.channelName?.trim() || null,
        id,
        lastActiveAt: activityAt,
        lastSummary: null,
        lastSummaryAt: null,
        preview,
        rootEventId,
        source,
      };
  const threads = [
    nextThread,
    ...state.threads.filter((thread) => thread.id !== id),
  ].slice(0, BESTIE_THREAD_MAX);
  return { ...state, threads };
}

export function removeBestieTrackedThread(
  state: BestieThreadState,
  id: string,
): BestieThreadState {
  const threads = state.threads.filter((thread) => thread.id !== id);
  if (threads.length === state.threads.length) return state;
  const pendingSummarize =
    state.pendingSummarize?.threadId === id ? null : state.pendingSummarize;
  return { ...state, pendingSummarize, threads };
}

export function beginBestieThreadSummarize(
  state: BestieThreadState,
  threadId: string,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieThreadState | null {
  if (state.pendingSummarize) return null;
  if (!state.threads.some((thread) => thread.id === threadId)) return null;
  return {
    ...state,
    pendingSummarize: {
      startedAt: nowSeconds,
      threadId,
      triggerMessageId: null,
    },
  };
}

export function clearBestieThreadSummarize(
  state: BestieThreadState,
): BestieThreadState {
  if (!state.pendingSummarize) return state;
  return { ...state, pendingSummarize: null };
}

/**
 * Finalize stuck summarize as failure text on the row, or clear if nothing posted.
 */
export function abandonBestieThreadSummarize(
  state: BestieThreadState,
  summary: string,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieThreadState {
  const pending = state.pendingSummarize;
  if (!pending) return state;
  if (!pending.triggerMessageId) {
    return { ...state, pendingSummarize: null };
  }
  return completeBestieThreadSummarize(state, summary, nowSeconds);
}

export function setBestieThreadSummarizeTrigger(
  state: BestieThreadState,
  triggerMessageId: string | null,
): BestieThreadState {
  if (!state.pendingSummarize) return state;
  return {
    ...state,
    pendingSummarize: { ...state.pendingSummarize, triggerMessageId },
  };
}

export function completeBestieThreadSummarize(
  state: BestieThreadState,
  summary: string,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieThreadState {
  const pending = state.pendingSummarize;
  if (!pending) return state;
  const trimmed = summary.trim();
  const threads = state.threads.map((thread) =>
    thread.id === pending.threadId
      ? {
          ...thread,
          lastActiveAt: nowSeconds,
          lastSummary: trimmed || thread.lastSummary || "(empty summarize reply)",
          lastSummaryAt: nowSeconds,
        }
      : thread,
  );
  return { ...state, pendingSummarize: null, threads };
}

export function sortedBestieThreads(
  state: BestieThreadState,
): BestieTrackedThread[] {
  return state.threads
    .slice()
    .sort((a, b) => b.lastActiveAt - a.lastActiveAt);
}
