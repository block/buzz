import * as React from "react";

import { bestieOwnerScopeKey } from "./bestieOwnerScope";

import {
  addBestieJob,
  EMPTY_BESTIE_JOB_STATE,
  markBestieJobFired,
  markBestieJobMessageProcessed,
  readBestieJobState,
  removeBestieJob,
  updateBestieJob,
  writeBestieJobState,
} from "./bestieJobStorage";
import { parseBestieJobActionsFromMessage } from "./parseBestieJobActions";
import {
  parseBestieUserJobIntent,
  type BestieUserJobIntent,
} from "./parseBestieUserJobIntent";
import { stripBestieOutboundHints } from "./bestieOutboundHints";
import type {
  BestieJobAddInput,
  BestieJobScope,
  BestieJobState,
  BestieJobUpdateInput,
} from "./bestieJobTypes";

type Listener = () => void;

const listenersByKey = new Map<string, Set<Listener>>();
const stateByKey = new Map<string, BestieJobState>();

function scopeKey(scope: BestieJobScope): string {
  return bestieOwnerScopeKey(scope);
}

function notify(key: string) {
  const listeners = listenersByKey.get(key);
  if (!listeners) return;
  for (const listener of listeners) listener();
}

function loadState(scope: BestieJobScope): BestieJobState {
  const key = scopeKey(scope);
  const cached = stateByKey.get(key);
  if (cached) return cached;
  const loaded = readBestieJobState(scope);
  stateByKey.set(key, loaded);
  return loaded;
}

function commit(scope: BestieJobScope, next: BestieJobState): BestieJobState {
  const key = scopeKey(scope);
  stateByKey.set(key, next);
  writeBestieJobState(scope, next);
  notify(key);
  return next;
}

export function getBestieJobState(scope: BestieJobScope): BestieJobState {
  return loadState(scope);
}

export function addBestieJobForScope(
  scope: BestieJobScope,
  input: BestieJobAddInput,
): BestieJobState {
  return commit(scope, addBestieJob(loadState(scope), input));
}

export function updateBestieJobForScope(
  scope: BestieJobScope,
  input: BestieJobUpdateInput,
): BestieJobState {
  return commit(scope, updateBestieJob(loadState(scope), input));
}

export function removeBestieJobForScope(
  scope: BestieJobScope,
  id: string,
): BestieJobState {
  return commit(scope, removeBestieJob(loadState(scope), id));
}

export function markBestieJobFiredForScope(
  scope: BestieJobScope,
  jobId: string,
  dueAt: number,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieJobState | null {
  const next = markBestieJobFired(loadState(scope), jobId, dueAt, nowSeconds);
  if (!next) return null;
  return commit(scope, next);
}

function applyUserJobIntent(
  state: BestieJobState,
  intent: BestieUserJobIntent,
  _messageId: string,
): { applied: number; state: BestieJobState } {
  // Chat path: never auto-create from NL schedule/approve — require confirmed fence.
  if (intent.op === "schedule-request" || intent.op === "approve-intent") {
    return { applied: 0, state };
  }
  if (intent.op === "remove-match") {
    const needle = intent.title.toLowerCase();
    const match = state.jobs.find(
      (job) =>
        job.enabled &&
        (job.title.toLowerCase() === needle ||
          job.title.toLowerCase().includes(needle)),
    );
    if (!match) return { applied: 0, state };
    return { applied: 1, state: removeBestieJob(state, match.id) };
  }
  // disable-match
  const needle = intent.title.toLowerCase();
  const match = state.jobs.find(
    (job) =>
      job.enabled &&
      (job.title.toLowerCase() === needle ||
        job.title.toLowerCase().includes(needle)),
  );
  if (!match) return { applied: 0, state };
  return {
    applied: 1,
    state: updateBestieJob(state, { enabled: false, id: match.id }),
  };
}

export function applyBestieJobActionsFromAgentMessage(
  scope: BestieJobScope,
  messageId: string,
  content: string,
): number {
  const current = loadState(scope);
  if (current.processedMessageIds.includes(messageId)) return 0;
  const actions = parseBestieJobActionsFromMessage(content);
  let next = markBestieJobMessageProcessed(current, messageId);
  if (actions.length === 0) {
    commit(scope, next);
    return 0;
  }
  let applied = 0;
  for (const action of actions) {
    if (action.op === "draft") {
      // Pending/draft fence — present plan in chat; do not create until confirmed add.
      continue;
    }
    if (action.op === "add") {
      const before = next;
      next = addBestieJob(next, {
        ...action.job,
        sourceMessageId: messageId,
      });
      if (next !== before) applied += 1;
      continue;
    }
    if (action.op === "update") {
      next = updateBestieJob(next, action);
      applied += 1;
      continue;
    }
    next = removeBestieJob(next, action.id);
    applied += 1;
  }
  commit(scope, next);
  return applied;
}

export function applyBestieJobIntentFromUserMessage(
  scope: BestieJobScope,
  messageId: string,
  content: string,
  nowMs = Date.now(),
): number {
  const current = loadState(scope);
  if (current.processedMessageIds.includes(messageId)) return 0;
  const intent = parseBestieUserJobIntent(
    stripBestieOutboundHints(content),
    nowMs,
  );
  const next = markBestieJobMessageProcessed(current, messageId);
  if (!intent) {
    commit(scope, next);
    return 0;
  }
  const result = applyUserJobIntent(next, intent, messageId);
  commit(scope, result.state);
  return result.applied;
}

function subscribe(scope: BestieJobScope, listener: Listener): () => void {
  const key = scopeKey(scope);
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
}

export function getBestieJobSnapshot(
  scope: BestieJobScope | null,
): BestieJobState {
  if (!scope) return EMPTY_BESTIE_JOB_STATE;
  return loadState(scope);
}

function getServerBestieJobSnapshot(): BestieJobState {
  return EMPTY_BESTIE_JOB_STATE;
}

export function useBestieJobs(scope: BestieJobScope | null): BestieJobState {
  const getSnapshot = React.useCallback(
    () => getBestieJobSnapshot(scope),
    [scope],
  );
  const subscribeScope = React.useCallback(
    (listener: Listener) => {
      if (!scope) return () => undefined;
      return subscribe(scope, listener);
    },
    [scope],
  );
  return React.useSyncExternalStore(
    subscribeScope,
    getSnapshot,
    getServerBestieJobSnapshot,
  );
}

export function __resetBestieJobStoreForTests(): void {
  stateByKey.clear();
  listenersByKey.clear();
}
