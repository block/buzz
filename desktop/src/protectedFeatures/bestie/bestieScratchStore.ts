import * as React from "react";

import { bestieOwnerScopeKey } from "./bestieOwnerScope";

import {
  addBestieScratchNote,
  EMPTY_BESTIE_SCRATCH_STATE,
  markBestieScratchMessageProcessed,
  readBestieScratchState,
  removeBestieScratchNote,
  updateBestieScratchNote,
  writeBestieScratchState,
} from "./bestieScratchStorage";
import {
  parseBestieScratchActionsFromMessage,
} from "./parseBestieScratchActions";
import {
  parseBestieUserScratchIntent,
  type BestieUserScratchIntent,
} from "./parseBestieUserScratchIntent";
import { stripBestieOutboundHints } from "./bestieOutboundHints";
import type {
  BestieScratchAddInput,
  BestieScratchScope,
  BestieScratchState,
  BestieScratchUpdateInput,
} from "./bestieScratchTypes";

type Listener = () => void;

const listenersByKey = new Map<string, Set<Listener>>();
const stateByKey = new Map<string, BestieScratchState>();

function scopeKey(scope: BestieScratchScope): string {
  return bestieOwnerScopeKey(scope);
}

function notify(key: string) {
  const listeners = listenersByKey.get(key);
  if (!listeners) return;
  for (const listener of listeners) listener();
}

function loadState(scope: BestieScratchScope): BestieScratchState {
  const key = scopeKey(scope);
  const cached = stateByKey.get(key);
  if (cached) return cached;
  const loaded = readBestieScratchState(scope);
  stateByKey.set(key, loaded);
  return loaded;
}

function commit(
  scope: BestieScratchScope,
  next: BestieScratchState,
): BestieScratchState {
  const key = scopeKey(scope);
  stateByKey.set(key, next);
  writeBestieScratchState(scope, next);
  notify(key);
  return next;
}

export function getBestieScratchState(
  scope: BestieScratchScope,
): BestieScratchState {
  return loadState(scope);
}

export function addBestieScratchNoteForScope(
  scope: BestieScratchScope,
  input: BestieScratchAddInput,
): BestieScratchState {
  return commit(scope, addBestieScratchNote(loadState(scope), input));
}

export function updateBestieScratchNoteForScope(
  scope: BestieScratchScope,
  input: BestieScratchUpdateInput,
): BestieScratchState {
  return commit(scope, updateBestieScratchNote(loadState(scope), input));
}

export function removeBestieScratchNoteForScope(
  scope: BestieScratchScope,
  id: string,
): BestieScratchState {
  return commit(scope, removeBestieScratchNote(loadState(scope), id));
}

function applyUserScratchIntent(
  state: BestieScratchState,
  intent: BestieUserScratchIntent,
  messageId: string,
): { applied: number; state: BestieScratchState } {
  if (intent.op === "add") {
    const before = state;
    const next = addBestieScratchNote(state, {
      ...intent.note,
      sourceMessageId: messageId,
    });
    return { applied: next !== before ? 1 : 0, state: next };
  }
  const needle = intent.title.toLowerCase();
  const match = state.notes.find(
    (note) =>
      note.title.toLowerCase() === needle ||
      note.title.toLowerCase().includes(needle),
  );
  if (!match) return { applied: 0, state };
  return { applied: 1, state: removeBestieScratchNote(state, match.id) };
}

export function applyBestieScratchActionsFromAgentMessage(
  scope: BestieScratchScope,
  messageId: string,
  content: string,
): number {
  const current = loadState(scope);
  if (current.processedMessageIds.includes(messageId)) return 0;
  const actions = parseBestieScratchActionsFromMessage(content);
  let next = markBestieScratchMessageProcessed(current, messageId);
  if (actions.length === 0) {
    commit(scope, next);
    return 0;
  }
  let applied = 0;
  for (const action of actions) {
    if (action.op === "add") {
      const before = next;
      next = addBestieScratchNote(next, {
        ...action.note,
        sourceMessageId: messageId,
      });
      if (next !== before) applied += 1;
      continue;
    }
    if (action.op === "update") {
      next = updateBestieScratchNote(next, action);
      applied += 1;
      continue;
    }
    next = removeBestieScratchNote(next, action.id);
    applied += 1;
  }
  commit(scope, next);
  return applied;
}

export function applyBestieScratchIntentFromUserMessage(
  scope: BestieScratchScope,
  messageId: string,
  content: string,
): number {
  const current = loadState(scope);
  if (current.processedMessageIds.includes(messageId)) return 0;
  const intent = parseBestieUserScratchIntent(stripBestieOutboundHints(content));
  const next = markBestieScratchMessageProcessed(current, messageId);
  if (!intent) {
    commit(scope, next);
    return 0;
  }
  const result = applyUserScratchIntent(next, intent, messageId);
  commit(scope, result.state);
  return result.applied;
}

function subscribe(scope: BestieScratchScope, listener: Listener): () => void {
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

export function getBestieScratchSnapshot(
  scope: BestieScratchScope | null,
): BestieScratchState {
  if (!scope) return EMPTY_BESTIE_SCRATCH_STATE;
  return loadState(scope);
}

function getServerBestieScratchSnapshot(): BestieScratchState {
  return EMPTY_BESTIE_SCRATCH_STATE;
}

export function useBestieScratch(
  scope: BestieScratchScope | null,
): BestieScratchState {
  const getSnapshot = React.useCallback(
    () => getBestieScratchSnapshot(scope),
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
    getServerBestieScratchSnapshot,
  );
}

export function __resetBestieScratchStoreForTests(): void {
  stateByKey.clear();
  listenersByKey.clear();
}
