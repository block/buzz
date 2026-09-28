import {
  bestieOwnerStorageKey,
  readOwnerScopedState,
  writeOwnerScopedState,
} from "./bestieOwnerScope";

import {
  DEFAULT_BESTIE_COFFEE_PREFS,
  deriveBestieCoffeeBrief,
  localDayKey,
  normalizeCoffeePrefs,
} from "./bestieCoffeeSchedule";
import { BESTIE_COFFEE_START_GRACE_SECONDS } from "./bestieCoffeeLive";
import type {
  BestieCoffeeAddEntryInput,
  BestieCoffeeEntry,
  BestieCoffeePendingRun,
  BestieCoffeePrefs,
  BestieCoffeeRunSource,
  BestieCoffeeScope,
  BestieCoffeeState,
} from "./bestieCoffeeTypes";

export const BESTIE_COFFEE_STORAGE_PREFIX = "buzz-bestie-coffee.v1";

/** Cap stored briefings so localStorage stays bounded. */
export const BESTIE_COFFEE_MAX_ENTRIES = 60;

/** Cap forgotten coffee trigger ids (deleted rows that must not rehydrate). */
export const BESTIE_COFFEE_MAX_FORGOTTEN_TRIGGERS = 200;

/** Owner+relay key — persists across Assistant agent reassignment. */
export function bestieCoffeeStorageKey(scope: BestieCoffeeScope): string {
  return bestieOwnerStorageKey(BESTIE_COFFEE_STORAGE_PREFIX, scope);
}

export const EMPTY_BESTIE_COFFEE_STATE: BestieCoffeeState = Object.freeze({
  entries: Object.freeze([]) as unknown as BestieCoffeeEntry[],
  forgottenTriggerIds: Object.freeze([]) as unknown as string[],
  lastScheduledDayKey: null,
  pendingRun: null,
  prefs: DEFAULT_BESTIE_COFFEE_PREFS,
  version: 1,
});

export function emptyBestieCoffeeState(): BestieCoffeeState {
  return EMPTY_BESTIE_COFFEE_STATE;
}

function isFiniteNonNegative(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

function parseEntry(value: unknown): BestieCoffeeEntry | null {
  if (typeof value !== "object" || value === null) return null;
  const record = value as Record<string, unknown>;
  if (
    typeof record.id !== "string" ||
    record.id.length === 0 ||
    !isFiniteNonNegative(record.ranAt) ||
    typeof record.brief !== "string" ||
    typeof record.fullOutput !== "string" ||
    (record.source !== "scheduled" && record.source !== "brew")
  ) {
    return null;
  }
  const triggerMessageId =
    typeof record.triggerMessageId === "string" &&
    record.triggerMessageId.length > 0
      ? record.triggerMessageId
      : null;
  const replyMessageId =
    typeof record.replyMessageId === "string" &&
    record.replyMessageId.length > 0
      ? record.replyMessageId
      : null;
  return {
    brief: record.brief.trim() || deriveBestieCoffeeBrief(record.fullOutput),
    fullOutput: record.fullOutput,
    id: record.id,
    ranAt: Math.floor(record.ranAt),
    replyMessageId,
    source: record.source,
    triggerMessageId,
  };
}

function parsePending(value: unknown): BestieCoffeePendingRun | null {
  if (typeof value !== "object" || value === null) return null;
  const record = value as Record<string, unknown>;
  if (
    (record.source !== "scheduled" && record.source !== "brew") ||
    !isFiniteNonNegative(record.startedAt)
  ) {
    return null;
  }
  const triggerMessageId =
    typeof record.triggerMessageId === "string" &&
    record.triggerMessageId.length > 0
      ? record.triggerMessageId
      : null;
  return {
    source: record.source,
    startedAt: Math.floor(record.startedAt),
    triggerMessageId,
  };
}

export function parseBestieCoffeeState(
  value: unknown,
): BestieCoffeeState | null {
  if (typeof value !== "object" || value === null) return null;
  const record = value as Record<string, unknown>;
  if (record.version !== 1 || !Array.isArray(record.entries)) return null;
  const entries: BestieCoffeeEntry[] = [];
  for (const entry of record.entries) {
    const parsed = parseEntry(entry);
    if (parsed) entries.push(parsed);
  }
  const prefs = normalizeCoffeePrefs(
    typeof record.prefs === "object" && record.prefs !== null
      ? (record.prefs as BestieCoffeePrefs)
      : undefined,
  );
  const lastScheduledDayKey =
    typeof record.lastScheduledDayKey === "string" &&
    /^\d{4}-\d{2}-\d{2}$/.test(record.lastScheduledDayKey)
      ? record.lastScheduledDayKey
      : null;
  const pendingRun = clearStalePendingOnLoad(
    parsePending(record.pendingRun),
    Math.floor(Date.now() / 1000),
  );
  const forgottenTriggerIds: string[] = [];
  if (Array.isArray(record.forgottenTriggerIds)) {
    for (const id of record.forgottenTriggerIds) {
      if (typeof id !== "string" || id.length === 0) continue;
      if (forgottenTriggerIds.includes(id)) continue;
      forgottenTriggerIds.push(id);
      if (forgottenTriggerIds.length >= BESTIE_COFFEE_MAX_FORGOTTEN_TRIGGERS) {
        break;
      }
    }
  }
  return {
    entries,
    forgottenTriggerIds,
    lastScheduledDayKey,
    pendingRun,
    prefs,
    version: 1,
  };
}

/**
 * Migrate stuck pending from prior sessions: any pending older than start
 * grace cannot still be mid-send, and ACP working state does not survive reload.
 */
export function clearStalePendingOnLoad(
  pending: BestieCoffeePendingRun | null,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieCoffeePendingRun | null {
  if (!pending) return null;
  const age = nowSeconds - pending.startedAt;
  if (age < 0) return null;
  if (age > BESTIE_COFFEE_START_GRACE_SECONDS) return null;
  return pending;
}


function maxDayKey(a: string | null, b: string | null): string | null {
  if (a == null) return b;
  if (b == null) return a;
  return a >= b ? a : b;
}

function mergeForgottenTriggerIds(
  into: readonly string[],
  from: readonly string[],
): string[] {
  const merged: string[] = [];
  for (const id of [...into, ...from]) {
    if (typeof id !== "string" || id.length === 0) continue;
    if (merged.includes(id)) continue;
    merged.push(id);
    if (merged.length >= BESTIE_COFFEE_MAX_FORGOTTEN_TRIGGERS) break;
  }
  return merged;
}

function mergeBestieCoffeeStates(
  into: BestieCoffeeState,
  from: BestieCoffeeState,
): BestieCoffeeState {
  const forgottenTriggerIds = mergeForgottenTriggerIds(
    into.forgottenTriggerIds,
    from.forgottenTriggerIds,
  );
  const forgotten = new Set(forgottenTriggerIds);
  const byId = new Map(into.entries.map((entry) => [entry.id, entry]));
  for (const entry of from.entries) {
    if (entry.triggerMessageId && forgotten.has(entry.triggerMessageId)) {
      continue;
    }
    const existing = byId.get(entry.id);
    if (!existing || entry.ranAt >= existing.ranAt) {
      byId.set(entry.id, entry);
    }
  }
  const entries = [...byId.values()].filter(
    (entry) =>
      !entry.triggerMessageId || !forgotten.has(entry.triggerMessageId),
  );
  return {
    entries,
    forgottenTriggerIds,
    lastScheduledDayKey: maxDayKey(
      into.lastScheduledDayKey,
      from.lastScheduledDayKey,
    ),
    pendingRun: into.pendingRun ?? from.pendingRun,
    prefs: into.prefs ?? from.prefs,
    version: 1,
  };
}

export function readBestieCoffeeState(scope: BestieCoffeeScope): BestieCoffeeState {
  return readOwnerScopedState({
    empty: emptyBestieCoffeeState,
    isEmpty: (state) =>
      state.entries.length === 0 &&
      state.pendingRun == null &&
      state.lastScheduledDayKey == null &&
      state.forgottenTriggerIds.length === 0,
    merge: mergeBestieCoffeeStates,
    parse: parseBestieCoffeeState,
    prefix: BESTIE_COFFEE_STORAGE_PREFIX,
    scope,
  });
}

export function writeBestieCoffeeState(
  scope: BestieCoffeeScope,
  state: BestieCoffeeState,
): void {
  writeOwnerScopedState(BESTIE_COFFEE_STORAGE_PREFIX, scope, state);
}

export function createBestieCoffeeEntryId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `coffee-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

export function setBestieCoffeePrefs(
  state: BestieCoffeeState,
  prefs: Partial<BestieCoffeePrefs>,
): BestieCoffeeState {
  return {
    ...state,
    prefs: normalizeCoffeePrefs({ ...state.prefs, ...prefs }),
  };
}

export function beginBestieCoffeeRun(
  state: BestieCoffeeState,
  source: BestieCoffeeRunSource,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieCoffeeState | null {
  if (state.pendingRun) return null;
  // Claim the local day *at begin* for scheduled runs so remount / wake /
  // presence re-ticks cannot re-fire after stale pending is cleared.
  let lastScheduledDayKey = state.lastScheduledDayKey;
  if (source === "scheduled") {
    const today = localDayKey(nowSeconds);
    if (state.lastScheduledDayKey === today) return null;
    const hasScheduledToday = state.entries.some(
      (entry) =>
        entry.source === "scheduled" && localDayKey(entry.ranAt) === today,
    );
    if (hasScheduledToday) return null;
    lastScheduledDayKey = today;
  }
  return {
    ...state,
    lastScheduledDayKey,
    pendingRun: {
      source,
      startedAt: nowSeconds,
      triggerMessageId: null,
    },
  };
}

/**
 * Past the local 08:00 fire window with no scheduled claim — mark the day
 * skipped so wake/remount/presence ticks cannot keep probing, without
 * posting a coffee turn. Brew remains allowed.
 */
export function claimMissedBestieCoffeeSchedule(
  state: BestieCoffeeState,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieCoffeeState {
  const today = localDayKey(nowSeconds);
  if (state.lastScheduledDayKey === today) return state;
  const hasScheduledToday = state.entries.some(
    (entry) =>
      entry.source === "scheduled" && localDayKey(entry.ranAt) === today,
  );
  if (hasScheduledToday) {
    return { ...state, lastScheduledDayKey: today };
  }
  return { ...state, lastScheduledDayKey: today };
}

export function setBestieCoffeePendingTrigger(
  state: BestieCoffeeState,
  triggerMessageId: string | null,
): BestieCoffeeState {
  if (!state.pendingRun) return state;
  return {
    ...state,
    pendingRun: { ...state.pendingRun, triggerMessageId },
  };
}

export function clearBestieCoffeePendingRun(
  state: BestieCoffeeState,
): BestieCoffeeState {
  if (!state.pendingRun) return state;
  return { ...state, pendingRun: null };
}

/**
 * Finalize a stuck pending run as a failure entry (so Coffee never stays on 👀
 * forever), or just clear when nothing was posted yet.
 */
export function abandonBestieCoffeeRun(
  state: BestieCoffeeState,
  fullOutput: string,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieCoffeeState {
  const pending = state.pendingRun;
  if (!pending) return state;
  // No trigger posted yet — nothing to show; drop the lock.
  if (!pending.triggerMessageId) {
    return { ...state, pendingRun: null };
  }
  // Already captured for this trigger.
  if (
    state.entries.some(
      (entry) => entry.triggerMessageId === pending.triggerMessageId,
    )
  ) {
    return { ...state, pendingRun: null };
  }
  return completeBestieCoffeeRun(
    state,
    {
      brief: "",
      fullOutput,
      replyMessageId: null,
      source: pending.source,
      triggerMessageId: pending.triggerMessageId,
    },
    nowSeconds,
  );
}

export function completeBestieCoffeeRun(
  state: BestieCoffeeState,
  input: BestieCoffeeAddEntryInput,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieCoffeeState {
  const source = state.pendingRun?.source ?? input.source;
  const brief =
    input.brief.trim() || deriveBestieCoffeeBrief(input.fullOutput);
  const entry: BestieCoffeeEntry = {
    brief,
    fullOutput: input.fullOutput,
    id: createBestieCoffeeEntryId(),
    ranAt: nowSeconds,
    replyMessageId: input.replyMessageId ?? null,
    source,
    triggerMessageId:
      input.triggerMessageId ?? state.pendingRun?.triggerMessageId ?? null,
  };
  const entries = [entry, ...state.entries].slice(0, BESTIE_COFFEE_MAX_ENTRIES);
  const lastScheduledDayKey =
    source === "scheduled"
      ? localDayKey(nowSeconds)
      : state.lastScheduledDayKey;
  // A fresh capture for this trigger supersedes any prior user delete tombstone.
  const forgottenTriggerIds =
    entry.triggerMessageId &&
    state.forgottenTriggerIds.includes(entry.triggerMessageId)
      ? state.forgottenTriggerIds.filter((id) => id !== entry.triggerMessageId)
      : state.forgottenTriggerIds;
  return {
    ...state,
    entries,
    forgottenTriggerIds,
    lastScheduledDayKey,
    pendingRun: null,
  };
}

export function removeBestieCoffeeEntry(
  state: BestieCoffeeState,
  id: string,
): BestieCoffeeState {
  const removed = state.entries.find((entry) => entry.id === id);
  if (!removed) return state;
  const entries = state.entries.filter((entry) => entry.id !== id);
  let forgottenTriggerIds = state.forgottenTriggerIds;
  if (removed.triggerMessageId) {
    forgottenTriggerIds = mergeForgottenTriggerIds(
      [removed.triggerMessageId],
      state.forgottenTriggerIds,
    );
  }
  return { ...state, entries, forgottenTriggerIds };
}

export function isBestieCoffeeBrewing(state: BestieCoffeeState): boolean {
  return state.pendingRun != null;
}
