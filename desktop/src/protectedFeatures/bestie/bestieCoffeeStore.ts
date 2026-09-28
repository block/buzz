import * as React from "react";

import { bestieOwnerScopeKey } from "./bestieOwnerScope";

import {
  abandonBestieCoffeeRun,
  beginBestieCoffeeRun,
  claimMissedBestieCoffeeSchedule,
  clearBestieCoffeePendingRun,
  completeBestieCoffeeRun,
  EMPTY_BESTIE_COFFEE_STATE,
  readBestieCoffeeState,
  removeBestieCoffeeEntry,
  setBestieCoffeePendingTrigger,
  setBestieCoffeePrefs,
  writeBestieCoffeeState,
} from "./bestieCoffeeStorage";
import {
  BESTIE_COFFEE_ABANDONED_OUTPUT,
  isBestieAcpStatusReactionContent,
  replyParentIdFromEventTags,
} from "./bestieCoffeeLive";
import type {
  BestieCoffeeAddEntryInput,
  BestieCoffeePrefs,
  BestieCoffeeRunSource,
  BestieCoffeeScope,
  BestieCoffeeState,
} from "./bestieCoffeeTypes";

type Listener = () => void;

const listenersByKey = new Map<string, Set<Listener>>();
const stateByKey = new Map<string, BestieCoffeeState>();

function scopeKey(scope: BestieCoffeeScope): string {
  return bestieOwnerScopeKey(scope);
}

function notify(key: string) {
  const listeners = listenersByKey.get(key);
  if (!listeners) return;
  for (const listener of listeners) listener();
}

function loadState(scope: BestieCoffeeScope): BestieCoffeeState {
  const key = scopeKey(scope);
  const cached = stateByKey.get(key);
  if (cached) return cached;
  const loaded = readBestieCoffeeState(scope);
  stateByKey.set(key, loaded);
  return loaded;
}

function commit(
  scope: BestieCoffeeScope,
  next: BestieCoffeeState,
): BestieCoffeeState {
  const key = scopeKey(scope);
  stateByKey.set(key, next);
  writeBestieCoffeeState(scope, next);
  notify(key);
  return next;
}

export function getBestieCoffeeState(
  scope: BestieCoffeeScope,
): BestieCoffeeState {
  return loadState(scope);
}

export function setBestieCoffeePrefsForScope(
  scope: BestieCoffeeScope,
  prefs: Partial<BestieCoffeePrefs>,
): BestieCoffeeState {
  return commit(scope, setBestieCoffeePrefs(loadState(scope), prefs));
}

/** Begin a brew/scheduled run; returns null if already brewing. */
export function beginBestieCoffeeRunForScope(
  scope: BestieCoffeeScope,
  source: BestieCoffeeRunSource,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieCoffeeState | null {
  const next = beginBestieCoffeeRun(loadState(scope), source, nowSeconds);
  if (!next) return null;
  return commit(scope, next);
}

export function claimMissedBestieCoffeeScheduleForScope(
  scope: BestieCoffeeScope,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieCoffeeState {
  return commit(
    scope,
    claimMissedBestieCoffeeSchedule(loadState(scope), nowSeconds),
  );
}

export function setBestieCoffeePendingTriggerForScope(
  scope: BestieCoffeeScope,
  triggerMessageId: string | null,
): BestieCoffeeState {
  return commit(
    scope,
    setBestieCoffeePendingTrigger(loadState(scope), triggerMessageId),
  );
}

export function clearBestieCoffeePendingRunForScope(
  scope: BestieCoffeeScope,
): BestieCoffeeState {
  return commit(scope, clearBestieCoffeePendingRun(loadState(scope)));
}

/** Finalize or clear a stale/abandoned pending brew so UI cannot stick on 👀. */
export function abandonBestieCoffeePendingForScope(
  scope: BestieCoffeeScope,
  fullOutput: string = BESTIE_COFFEE_ABANDONED_OUTPUT,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieCoffeeState {
  return commit(
    scope,
    abandonBestieCoffeeRun(loadState(scope), fullOutput, nowSeconds),
  );
}

export function completeBestieCoffeeRunForScope(
  scope: BestieCoffeeScope,
  input: BestieCoffeeAddEntryInput,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieCoffeeState {
  return commit(
    scope,
    completeBestieCoffeeRun(loadState(scope), input, nowSeconds),
  );
}

export function removeBestieCoffeeEntryForScope(
  scope: BestieCoffeeScope,
  id: string,
): BestieCoffeeState {
  return commit(scope, removeBestieCoffeeEntry(loadState(scope), id));
}

/**
 * Capture an agent reply into a Coffee tab entry.
 *
 * Records **all** outcomes (briefs, failures, NCP-disabled, errors) when the
 * agent message replies to a coffee trigger — never Thread Summarize / job /
 * other Assistant turns. Prefer pendingRun + bound triggerMessageId; also
 * fold replies to unmatched coffee triggers when pending was lost.
 */
/** True when a Coffee row should accept a better in-thread reply (Path C). */
export function isBestieCoffeeStubEntry(entry: {
  brief: string;
  fullOutput: string;
  replyMessageId: string | null;
}): boolean {
  if (!entry.replyMessageId) return true;
  if (!entry.fullOutput.trim()) return true;
  if (!entry.brief.trim()) return true;
  if (entry.fullOutput.trim() === BESTIE_COFFEE_ABANDONED_OUTPUT) return true;
  // Prior bug: kind:7 👀/💬 was captured as the brew — still treat as open stub.
  if (isBestieAcpStatusReactionContent(entry.fullOutput)) return true;
  if (isBestieAcpStatusReactionContent(entry.brief)) return true;
  return false;
}

export function applyBestieCoffeeAgentReply(
  scope: BestieCoffeeScope,
  messageId: string,
  content: string,
  createdAtSeconds: number,
  tags?: readonly (readonly string[])[] | null,
  /**
   * Optional map of coffee trigger message id → source, for capture when
   * pendingRun was cleared or never set (manual /hula-coffee).
   */
  unmatchedCoffeeTriggers?: ReadonlyMap<string, "scheduled" | "brew">,
): boolean {
  const current = loadState(scope);
  const trimmed = content.trim();
  if (!trimmed) return false;
  // Never finalize Coffee from ACP seen/working reactions (kind:7 👀/💬).
  if (isBestieAcpStatusReactionContent(trimmed)) return false;

  // Already captured this agent message.
  if (current.entries.some((entry) => entry.replyMessageId === messageId)) {
    return false;
  }

  const parentId = replyParentIdFromEventTags(tags);
  const pending = current.pendingRun;
  const sameEventId = (a: string | null | undefined, b: string | null | undefined) =>
    Boolean(a && b && a.toLowerCase() === b.toLowerCase());

  const finalizeForTrigger = (
    triggerMessageId: string,
    source: "scheduled" | "brew",
    ranAt: number,
    clearPending: boolean,
  ): boolean => {
    // User deleted this brew — do not rehydrate from chat history.
    if (
      current.forgottenTriggerIds.some((id) => sameEventId(id, triggerMessageId))
    ) {
      return false;
    }
    const existing = loadState(scope).entries.find((entry) =>
      sameEventId(entry.triggerMessageId, triggerMessageId),
    );
    if (existing && !isBestieCoffeeStubEntry(existing)) {
      // Already have a real capture for this trigger — drop leftover pending.
      if (clearPending && loadState(scope).pendingRun) {
        clearBestieCoffeePendingRunForScope(scope);
      }
      return false;
    }
    if (existing && isBestieCoffeeStubEntry(existing)) {
      // Path C upgrade: replace empty / abandoned / unbound stub with full reply.
      const without = {
        ...loadState(scope),
        entries: loadState(scope).entries.filter(
          (entry) => entry.id !== existing.id,
        ),
        pendingRun: clearPending ? null : loadState(scope).pendingRun,
      };
      commit(
        scope,
        completeBestieCoffeeRun(
          without,
          {
            brief: "",
            fullOutput: trimmed,
            replyMessageId: messageId,
            source: existing.source ?? source,
            triggerMessageId,
          },
          ranAt,
        ),
      );
      return true;
    }
    const stateForComplete = clearPending
      ? { ...loadState(scope), pendingRun: null }
      : loadState(scope);
    commit(
      scope,
      completeBestieCoffeeRun(
        stateForComplete,
        {
          brief: "",
          fullOutput: trimmed,
          replyMessageId: messageId,
          source,
          triggerMessageId,
        },
        ranAt,
      ),
    );
    return true;
  };

  // Path A: pending run with bound trigger — in-thread reply to that root.
  if (pending?.triggerMessageId) {
    if (createdAtSeconds + 5 < pending.startedAt) return false;
    if (sameEventId(parentId, pending.triggerMessageId)) {
      return finalizeForTrigger(
        pending.triggerMessageId,
        pending.source,
        Math.max(createdAtSeconds, pending.startedAt),
        true,
      );
    }
    // Bound to a different root (e.g. newer duplicate fire) — fall through so
    // Path C can still fill the stub for the trigger the agent actually replied to.
  }

  const unmatchedSourceFor = (
    triggerId: string,
  ): "scheduled" | "brew" | undefined => {
    if (!unmatchedCoffeeTriggers) return undefined;
    for (const [id, source] of unmatchedCoffeeTriggers) {
      if (sameEventId(id, triggerId)) return source;
    }
    return undefined;
  };

  // Path B: pending without trigger — bind from this reply's coffee parent, then capture.
  if (pending && !pending.triggerMessageId) {
    if (parentId && unmatchedSourceFor(parentId) != null) {
      setBestieCoffeePendingTriggerForScope(scope, parentId);
      return finalizeForTrigger(
        parentId,
        pending.source,
        Math.max(createdAtSeconds, pending.startedAt),
        true,
      );
    }
    // Not a coffee-parent reply yet — wait for bind (avoid stealing summarize/job).
    return false;
  }

  // Path C: capture / upgrade reply to an unmatched coffee trigger OR open stub
  // (stub trigger may have aged out of the roots-only channel window).
  if (parentId) {
    const stubForParent = loadState(scope).entries.find(
      (entry) =>
        sameEventId(entry.triggerMessageId, parentId) &&
        isBestieCoffeeStubEntry(entry),
    );
    const unmatchedSource = unmatchedSourceFor(parentId);
    if (stubForParent || unmatchedSource != null) {
      const source =
        stubForParent?.source ??
        unmatchedSource ??
        loadState(scope).entries.find((entry) =>
          sameEventId(entry.triggerMessageId, parentId),
        )?.source ??
        "brew";
      return finalizeForTrigger(
        stubForParent?.triggerMessageId ?? parentId,
        source,
        createdAtSeconds,
        true,
      );
    }
  }

  // Path D: pending brew whose agent reply landed top-level (no e-tag) — common
  // when ACP omitted --reply-to on the first Assistant DM turn. Fold into the
  // pending trigger so Coffee cannot stick on 👀 after a finished brew.
  if (pending?.triggerMessageId && !parentId) {
    if (createdAtSeconds + 5 >= pending.startedAt) {
      return finalizeForTrigger(
        pending.triggerMessageId,
        pending.source,
        Math.max(createdAtSeconds, pending.startedAt),
        true,
      );
    }
  }

  return false;
}

export function useBestieCoffee(
  scope: BestieCoffeeScope | null,
): BestieCoffeeState {
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
    if (!scope) return EMPTY_BESTIE_COFFEE_STATE;
    return loadState(scope);
  }, [scope]);
  return React.useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}

/** Test helper: drop cached in-memory coffee state. */
export function __resetBestieCoffeeStoreForTests(): void {
  stateByKey.clear();
  listenersByKey.clear();
}

