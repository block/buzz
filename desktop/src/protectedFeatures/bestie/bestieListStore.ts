import * as React from "react";

import { bestieOwnerScopeKey } from "./bestieOwnerScope";

import {
  addBestieListItem,
  dismissBestieReminderItems,
  EMPTY_BESTIE_LIST_STATE,
  markBestieListMessageProcessed,
  readBestieListState,
  removeBestieListItem,
  reorderBestieTodos,
  snoozeBestieListItems,
  toggleBestieListItemStarred,
  updateBestieListItem,
  updateBestieListItemStatus,
  writeBestieListState,
} from "./bestieListStorage";
import { parseBestieListActionsFromMessage } from "./parseBestieListActions";
import {
  parseBestieReminderMeridiemReply,
  parseBestieUserListIntent,
  reconcileReminderDueAtWithStatedMeridiem,
  resolveBestieBareClockDueAt,
  type BestieUserListIntent,
} from "./parseBestieUserListIntent";
import { stripBestieOutboundHints } from "./bestieOutboundHints";
import type {
  BestieListAddInput,
  BestieListItem,
  BestieListKind,
  BestieListScope,
  BestieListState,
  BestieListTodoUpdateInput,
} from "./bestieListTypes";

type Listener = () => void;

const listenersByKey = new Map<string, Set<Listener>>();
const stateByKey = new Map<string, BestieListState>();

function scopeKey(scope: BestieListScope): string {
  return bestieOwnerScopeKey(scope);
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

export function snoozeBestieListItemsForScope(
  scope: BestieListScope,
  ids: string[],
  deltaSeconds: number,
): BestieListState {
  return commit(
    scope,
    snoozeBestieListItems(loadState(scope), ids, deltaSeconds),
  );
}

export function dismissBestieReminderItemsForScope(
  scope: BestieListScope,
  ids: string[],
): BestieListState {
  return commit(scope, dismissBestieReminderItems(loadState(scope), ids));
}

export function toggleBestieListItemStarredForScope(
  scope: BestieListScope,
  id: string,
): BestieListState {
  return commit(scope, toggleBestieListItemStarred(loadState(scope), id));
}

export function updateBestieListItemForScope(
  scope: BestieListScope,
  input: BestieListTodoUpdateInput,
): BestieListState {
  return commit(scope, updateBestieListItem(loadState(scope), input));
}

export function reorderBestieTodosForScope(
  scope: BestieListScope,
  options: {
    dayKey?: string | null;
    orderedIds: string[];
    starred: boolean;
  },
): BestieListState {
  return commit(scope, reorderBestieTodos(loadState(scope), options));
}

/**
 * Apply structured list actions from an agent message once.
 * Returns how many mutations landed (0 if already processed / no actions).
 */
export function applyBestieListActionsFromAgentMessage(
  scope: BestieListScope,
  messageId: string,
  content: string,
  nowMs = Date.now(),
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
  const pending = next.pendingReminderConfirm;
  for (const action of actions) {
    if (action.op === "add") {
      for (const item of action.items) {
        let addInput = {
          ...item,
          sourceMessageId: messageId,
        };
        if (item.kind === "reminder") {
          const bareClock =
            pending &&
            item.text.trim().toLowerCase() === pending.text.toLowerCase()
              ? pending.bareClock
              : pending?.bareClock ?? null;
          const reconciledDue = reconcileReminderDueAtWithStatedMeridiem(
            item.dueAt ?? null,
            content,
            bareClock,
            nowMs,
          );
          if (reconciledDue != null && reconciledDue !== (item.dueAt ?? null)) {
            addInput = { ...addInput, dueAt: reconciledDue };
          }
        }
        const before = next;
        next = addBestieListItem(
          next,
          addInput,
          Math.floor(nowMs / 1000),
        );
        if (next !== before) applied += 1;
        // Clear pending when a matching reminder lands (fence or reconciled).
        if (
          next.pendingReminderConfirm &&
          item.kind === "reminder" &&
          item.text.trim().toLowerCase() ===
            next.pendingReminderConfirm.text.toLowerCase()
        ) {
          next = { ...next, pendingReminderConfirm: null };
        }
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
    return (
      item.text.toLowerCase() === needle ||
      item.text.toLowerCase().includes(needle)
    );
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
  nowMs: number,
): { applied: number; state: BestieListState } {
  // Bare clock without am/pm — stash pending; create after AM/PM reply (or fixed fence).
  if (intent.op === "reminder-confirm-needed") {
    return {
      applied: 0,
      state: {
        ...state,
        pendingReminderConfirm: {
          bareClock: intent.bareClock,
          createdAt: nowMs,
          sourceMessageId: messageId,
          text: intent.text,
        },
      },
    };
  }
  if (intent.op === "add") {
    let next = state;
    let applied = 0;
    const nowSeconds = Math.floor(nowMs / 1000);
    for (const item of intent.items) {
      const before = next;
      next = addBestieListItem(
        next,
        {
          ...item,
          sourceMessageId: messageId,
        },
        nowSeconds,
      );
      if (next !== before) applied += 1;
    }
    // A fully-specified add clears any stale pending for the same text.
    if (
      next.pendingReminderConfirm &&
      intent.items.some(
        (item) =>
          item.kind === "reminder" &&
          item.text.trim().toLowerCase() ===
            next.pendingReminderConfirm!.text.toLowerCase(),
      )
    ) {
      next = { ...next, pendingReminderConfirm: null };
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
 * Apply a short AM/PM reply against pending bare-clock confirm.
 * Resolves dueAt client-side so agent fence epoch cannot invent the wrong half-day.
 */
function applyPendingMeridiemConfirm(
  state: BestieListState,
  messageId: string,
  content: string,
  nowMs: number,
): { applied: number; state: BestieListState } | null {
  const pending = state.pendingReminderConfirm;
  if (!pending) return null;
  const reply = parseBestieReminderMeridiemReply(content);
  if (!reply) return null;
  let bareClock = pending.bareClock;
  if (reply.hour != null && reply.hour >= 1 && reply.hour <= 12) {
    bareClock = {
      ...bareClock,
      hour: reply.hour,
      minute: reply.minute ?? bareClock.minute,
    };
  }
  const dueAt = resolveBestieBareClockDueAt(bareClock, reply.meridiem, nowMs);
  let next: BestieListState = {
    ...state,
    pendingReminderConfirm: null,
  };
  const before = next;
  next = addBestieListItem(
    next,
    {
      dueAt,
      kind: "reminder",
      sourceMessageId: messageId,
      text: pending.text,
    },
    Math.floor(nowMs / 1000),
  );
  return {
    applied: next !== before ? 1 : 0,
    state: next,
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
  const userContent = stripBestieOutboundHints(content);
  let next = markBestieListMessageProcessed(current, messageId);

  // Drop stale pendings (2h) so unrelated chat is not trapped.
  if (
    next.pendingReminderConfirm &&
    nowMs - next.pendingReminderConfirm.createdAt > 2 * 60 * 60 * 1000
  ) {
    next = { ...next, pendingReminderConfirm: null };
  }

  // Prefer client resolve of AM/PM reply against pending bare-clock confirm.
  const meridiemResult = applyPendingMeridiemConfirm(
    next,
    messageId,
    userContent,
    nowMs,
  );
  if (meridiemResult) {
    commit(scope, meridiemResult.state);
    return meridiemResult.applied;
  }

  const intent = parseBestieUserListIntent(userContent, nowMs);
  if (!intent) {
    commit(scope, next);
    return 0;
  }
  const result = applyUserIntent(next, intent, messageId, nowMs);
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
