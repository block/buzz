import * as React from "react";

import {
  addBestieListItem,
  EMPTY_BESTIE_LIST_STATE,
  markBestieListMessageProcessed,
  readBestieListState,
  removeBestieListItem,
  updateBestieListItemStatus,
  writeBestieListState,
} from "./bestieListStorage";
import { parseBestieListActionsFromMessage } from "./parseBestieListActions";
import {
  parseBestieUserListIntent,
  type BestieUserListIntent,
} from "./parseBestieUserListIntent";
import type {
  BestieListAddInput,
  BestieListItem,
  BestieListKind,
  BestieListScope,
  BestieListState,
} from "./bestieListTypes";

type Listener = () => void;

const listenersByKey = new Map<string, Set<Listener>>();
const stateByKey = new Map<string, BestieListState>();

function scopeKey(scope: BestieListScope): string {
  return [
    scope.relayUrl.trim().toLowerCase(),
    scope.ownerPubkey.toLowerCase(),
    scope.agentPubkey.toLowerCase(),
  ].join(":");
}

function notify(key: string) {
  const listeners = listenersByKey.get(key);
  if (!listeners) return;
  for (const listener of listeners) listener();
}

function loadState(scope: BestieListScope): BestieListState {
  const key = scopeKey(scope);
  const cached = stateByKey.get(key);
  if (cached) return cached;
  const loaded = readBestieListState(scope);
  stateByKey.set(key, loaded);
  return loaded;
}

function commit(
  scope: BestieListScope,
  next: BestieListState,
): BestieListState {
  const key = scopeKey(scope);
  stateByKey.set(key, next);
  writeBestieListState(scope, next);
  notify(key);
  return next;
}

export function getBestieListState(scope: BestieListScope): BestieListState {
  return loadState(scope);
}

export function addBestieListItemForScope(
  scope: BestieListScope,
  input: BestieListAddInput,
): BestieListState {
  return commit(scope, addBestieListItem(loadState(scope), input));
}

export function setBestieListItemStatusForScope(
  scope: BestieListScope,
  id: string,
  status: "open" | "done",
): BestieListState {
  return commit(
    scope,
    updateBestieListItemStatus(loadState(scope), id, status),
  );
}

export function removeBestieListItemForScope(
  scope: BestieListScope,
  id: string,
): BestieListState {
  return commit(scope, removeBestieListItem(loadState(scope), id));
}

/**
 * Apply structured list actions from an agent message once.
 * Returns how many mutations landed (0 if already processed / no actions).
 */
export function applyBestieListActionsFromAgentMessage(
  scope: BestieListScope,
  messageId: string,
  content: string,
): number {
  const current = loadState(scope);
  if (current.processedMessageIds.includes(messageId)) return 0;
  const actions = parseBestieListActionsFromMessage(content);
  let next = markBestieListMessageProcessed(current, messageId);
  if (actions.length === 0) {
    commit(scope, next);
    return 0;
  }
  let applied = 0;
  for (const action of actions) {
    if (action.op === "add") {
      for (const item of action.items) {
        next = addBestieListItem(next, {
          ...item,
          sourceMessageId: messageId,
        });
        applied += 1;
      }
      continue;
    }
    if (action.op === "complete") {
      next = updateBestieListItemStatus(next, action.id, "done");
      applied += 1;
      continue;
    }
    next = removeBestieListItem(next, action.id);
    applied += 1;
  }
  commit(scope, next);
  return applied;
}

function subscribe(scope: BestieListScope, listener: Listener): () => void {
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


function findOpenItemByText(
  state: BestieListState,
  text: string,
  kind?: BestieListKind,
): BestieListItem | null {
  const needle = text.trim().toLowerCase();
  if (!needle) return null;
  const matches = state.items.filter((item) => {
    if (item.status !== "open") return false;
    if (kind && item.kind !== kind) return false;
    return item.text.toLowerCase() === needle || item.text.toLowerCase().includes(needle);
  });
  if (matches.length === 0) return null;
  // Prefer exact match, then shortest text (most specific).
  matches.sort((a, b) => {
    const aExact = a.text.toLowerCase() === needle ? 0 : 1;
    const bExact = b.text.toLowerCase() === needle ? 0 : 1;
    if (aExact !== bExact) return aExact - bExact;
    return a.text.length - b.text.length;
  });
  return matches[0] ?? null;
}

function applyUserIntent(
  state: BestieListState,
  intent: BestieUserListIntent,
  messageId: string,
): { applied: number; state: BestieListState } {
  if (intent.op === "add") {
    let next = state;
    let applied = 0;
    for (const item of intent.items) {
      next = addBestieListItem(next, {
        ...item,
        sourceMessageId: messageId,
      });
      applied += 1;
    }
    return { applied, state: next };
  }
  const match = findOpenItemByText(state, intent.text, intent.kind);
  if (!match) return { applied: 0, state };
  if (intent.op === "complete-match") {
    return {
      applied: 1,
      state: updateBestieListItemStatus(state, match.id, "done"),
    };
  }
  return {
    applied: 1,
    state: removeBestieListItem(state, match.id),
  };
}

/**
 * Apply natural-language list intents from a *user* Bestie message once.
 * Returns how many mutations landed (0 if already processed / no intent).
 */
export function applyBestieListIntentFromUserMessage(
  scope: BestieListScope,
  messageId: string,
  content: string,
  nowMs = Date.now(),
): number {
  const current = loadState(scope);
  if (current.processedMessageIds.includes(messageId)) return 0;
  const intent = parseBestieUserListIntent(content, nowMs);
  let next = markBestieListMessageProcessed(current, messageId);
  if (!intent) {
    commit(scope, next);
    return 0;
  }
  const result = applyUserIntent(next, intent, messageId);
  commit(scope, result.state);
  return result.applied;
}

/**
 * Snapshot for React / tests. Null scope returns the shared empty constant so
 * useSyncExternalStore does not see a new object every getSnapshot call
 * (Maximum update depth exceeded).
 */
export function getBestieListSnapshot(
  scope: BestieListScope | null,
): BestieListState {
  if (!scope) return EMPTY_BESTIE_LIST_STATE;
  return loadState(scope);
}

function getServerBestieListSnapshot(): BestieListState {
  return EMPTY_BESTIE_LIST_STATE;
}

/** React hook for the scoped Bestie reminders/todos list. */
export function useBestieList(scope: BestieListScope | null): BestieListState {
  const getSnapshot = React.useCallback(
    () => getBestieListSnapshot(scope),
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
    getServerBestieListSnapshot,
  );
}

/** Test helper: drop cached in-memory state. */
export function __resetBestieListStoreForTests(): void {
  stateByKey.clear();
  listenersByKey.clear();
}
