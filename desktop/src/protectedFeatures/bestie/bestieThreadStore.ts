import * as React from "react";

import { bestieOwnerScopeKey } from "./bestieOwnerScope";

import { bestieThreadId } from "./bestieThreadProtocol";
import { replyParentIdFromEventTags } from "./bestieCoffeeLive";
import {
  abandonBestieThreadSummarize,
  beginBestieThreadSummarize,
  clearBestieThreadSummarize,
  completeBestieThreadSummarize,
  EMPTY_BESTIE_THREAD_STATE,
  readBestieThreadState,
  removeBestieTrackedThread,
  setBestieThreadSummarizeTrigger,
  upsertBestieTrackedThread,
  writeBestieThreadState,
} from "./bestieThreadStorage";
import { BESTIE_THREAD_SUMMARIZE_ABANDONED_OUTPUT } from "./bestieThreadSummarizeLive";
import type {
  BestieThreadScope,
  BestieThreadState,
  BestieThreadUpsertInput,
} from "./bestieThreadTypes";

type Listener = () => void;

const listenersByKey = new Map<string, Set<Listener>>();
const stateByKey = new Map<string, BestieThreadState>();

function scopeKey(scope: BestieThreadScope): string {
  return bestieOwnerScopeKey(scope);
}

function notify(key: string) {
  const listeners = listenersByKey.get(key);
  if (!listeners) return;
  for (const listener of listeners) listener();
}

function loadState(scope: BestieThreadScope): BestieThreadState {
  const key = scopeKey(scope);
  const cached = stateByKey.get(key);
  if (cached) return cached;
  const loaded = readBestieThreadState(scope);
  stateByKey.set(key, loaded);
  return loaded;
}

function commit(
  scope: BestieThreadScope,
  next: BestieThreadState,
): BestieThreadState {
  const key = scopeKey(scope);
  stateByKey.set(key, next);
  writeBestieThreadState(scope, next);
  notify(key);
  return next;
}

export function getBestieThreadState(
  scope: BestieThreadScope,
): BestieThreadState {
  return loadState(scope);
}

export function upsertBestieTrackedThreadForScope(
  scope: BestieThreadScope,
  input: BestieThreadUpsertInput,
): BestieThreadState {
  return commit(scope, upsertBestieTrackedThread(loadState(scope), input));
}

export function removeBestieTrackedThreadForScope(
  scope: BestieThreadScope,
  id: string,
): BestieThreadState {
  return commit(scope, removeBestieTrackedThread(loadState(scope), id));
}

export function beginBestieThreadSummarizeForScope(
  scope: BestieThreadScope,
  threadId: string,
): BestieThreadState | null {
  const next = beginBestieThreadSummarize(loadState(scope), threadId);
  if (!next) return null;
  return commit(scope, next);
}

export function clearBestieThreadSummarizeForScope(
  scope: BestieThreadScope,
): BestieThreadState {
  return commit(scope, clearBestieThreadSummarize(loadState(scope)));
}

/** Finalize or clear a stale summarize pending so rows cannot stick on Summarizing… */
export function abandonBestieThreadSummarizeForScope(
  scope: BestieThreadScope,
  summary: string = BESTIE_THREAD_SUMMARIZE_ABANDONED_OUTPUT,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieThreadState {
  return commit(
    scope,
    abandonBestieThreadSummarize(loadState(scope), summary, nowSeconds),
  );
}

export function setBestieThreadSummarizeTriggerForScope(
  scope: BestieThreadScope,
  triggerMessageId: string | null,
): BestieThreadState {
  return commit(
    scope,
    setBestieThreadSummarizeTrigger(loadState(scope), triggerMessageId),
  );
}

/**
 * Capture an agent reply onto the pending Threads row (success, failures,
 * errors). Only messages that reply to the summarize trigger parent count —
 * Coffee / other Assistant turns must not steal or fake a summarize outcome.
 */
export function applyBestieThreadSummarizeReply(
  scope: BestieThreadScope,
  _messageId: string,
  content: string,
  createdAtSeconds: number,
  tags?: readonly (readonly string[])[] | null,
  /**
   * Unmatched summarize prompt ids → tracked thread id (when pending was lost).
   */
  unmatchedSummarizeTriggers?: ReadonlyMap<string, string>,
): boolean {
  const current = loadState(scope);
  const trimmed = content.trim();
  if (!trimmed) return false;

  const parentId = replyParentIdFromEventTags(tags);
  const pending = current.pendingSummarize;

  if (pending?.triggerMessageId) {
    if (createdAtSeconds + 5 < pending.startedAt) return false;
    if (parentId !== pending.triggerMessageId) return false;
    commit(
      scope,
      completeBestieThreadSummarize(current, trimmed, createdAtSeconds),
    );
    return true;
  }

  // Wait for trigger bind so Coffee replies cannot complete summarize early.
  if (pending && !pending.triggerMessageId) {
    return false;
  }

  // No pending — still fold replies to unmatched summarize prompts onto the row.
  // Overwrites a prior abandon/timeout lastSummary when the real in-thread reply
  // arrives (channel window is roots-only; replies live in thread-replies cache).
  if (parentId && unmatchedSummarizeTriggers?.has(parentId)) {
    const threadId = unmatchedSummarizeTriggers.get(parentId);
    if (!threadId) return false;
    const thread = current.threads.find((entry) => entry.id === threadId);
    if (!thread) return false;
    if (
      thread.lastSummary &&
      thread.lastSummary !== BESTIE_THREAD_SUMMARIZE_ABANDONED_OUTPUT
    ) {
      // Already have a real summary for this thread from a newer run — skip.
      // (Abandoned timeout text is replaceable.)
      if (
        thread.lastSummaryAt != null &&
        createdAtSeconds + 5 < thread.lastSummaryAt
      ) {
        return false;
      }
    }
    const withPending = {
      ...current,
      pendingSummarize: {
        startedAt: createdAtSeconds,
        threadId,
        triggerMessageId: parentId,
      },
    };
    commit(
      scope,
      completeBestieThreadSummarize(withPending, trimmed, createdAtSeconds),
    );
    return true;
  }

  return false;
}


export function syncBestieParticipatingThreadsForScope(
  scope: BestieThreadScope,
  inputs: readonly BestieThreadUpsertInput[],
): BestieThreadState {
  let next = loadState(scope);
  const known = new Set(next.threads.map((thread) => thread.id));
  let changed = false;
  for (const input of inputs) {
    const channelId = input.channelId.trim();
    const rootEventId = input.rootEventId.trim();
    if (!channelId || !rootEventId) continue;
    const id = bestieThreadId(channelId, rootEventId);
    if (known.has(id)) {
      const existing = next.threads.find((thread) => thread.id === id);
      if (!existing) continue;
      const activityAt =
        typeof input.lastActiveAt === "number" &&
        Number.isFinite(input.lastActiveAt)
          ? Math.floor(input.lastActiveAt)
          : null;
      const newerActivity =
        activityAt != null && activityAt > existing.lastActiveAt;
      const promoteAskToAgent =
        existing.source === "ask" && (input.source ?? "agent") === "agent";
      if (newerActivity || promoteAskToAgent) {
        next = upsertBestieTrackedThread(next, {
          ...input,
          source: promoteAskToAgent ? "agent" : existing.source,
        });
        changed = true;
      }
      continue;
    }
    next = upsertBestieTrackedThread(next, {
      ...input,
      source: input.source ?? "agent",
    });
    known.add(id);
    changed = true;
  }
  if (!changed) return next;
  return commit(scope, next);
}

export function useBestieThreads(
  scope: BestieThreadScope | null,
): BestieThreadState {
  const key = scope ? scopeKey(scope) : null;
  const subscribe = React.useCallback(
    (listener: Listener) => {
      if (!key) return () => {};
      let set = listenersByKey.get(key);
      if (!set) {
        set = new Set();
        listenersByKey.set(key, set);
      }
      set.add(listener);
      return () => {
        set?.delete(listener);
        if (set && set.size === 0) listenersByKey.delete(key);
      };
    },
    [key],
  );
  const getSnapshot = React.useCallback(() => {
    if (!scope) return EMPTY_BESTIE_THREAD_STATE;
    return loadState(scope);
  }, [scope]);
  return React.useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}

/** Test helper: drop cached in-memory thread state. */
export function __resetBestieThreadStoreForTests(): void {
  stateByKey.clear();
  listenersByKey.clear();
}
