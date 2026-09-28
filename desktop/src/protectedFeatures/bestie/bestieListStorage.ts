import {
  bestieOwnerStorageKey,
  readOwnerScopedState,
  writeOwnerScopedState,
} from "./bestieOwnerScope";

import type {
  BestieListAddInput,
  BestieListItem,
  BestieListKind,
  BestieListScope,
  BestieListState,
  BestieListTodoUpdateInput,
  BestiePendingReminderConfirm,
  BestieReminderRepeat,
} from "./bestieListTypes";

/** Local YYYY-MM-DD for todo day grouping. */
export function localDayKeyFromSeconds(seconds: number): string {
  const date = new Date(seconds * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

export function todayLocalDayKey(
  nowSeconds = Math.floor(Date.now() / 1000),
): string {
  return localDayKeyFromSeconds(nowSeconds);
}

export const BESTIE_LIST_STORAGE_PREFIX = "buzz-bestie-list.v1";

/** Owner+relay key — persists across Assistant agent reassignment. */
export function bestieListStorageKey(scope: BestieListScope): string {
  return bestieOwnerStorageKey(BESTIE_LIST_STORAGE_PREFIX, scope);
}

function isFiniteNonNegative(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

function isKind(value: unknown): value is BestieListKind {
  return value === "todo" || value === "reminder";
}

function parseReminderRepeat(value: unknown): BestieReminderRepeat | null {
  if (typeof value !== "object" || value === null) return null;
  const record = value as Record<string, unknown>;
  if (record.kind === "daily") return { kind: "daily" };
  if (record.kind === "weekly") {
    const weekday =
      typeof record.weekday === "number" && Number.isFinite(record.weekday)
        ? Math.floor(record.weekday)
        : null;
    if (weekday == null) return null;
    return { kind: "weekly", weekday: ((weekday % 7) + 7) % 7 };
  }
  return null;
}


function parsePendingReminderConfirm(
  value: unknown,
): BestiePendingReminderConfirm | null {
  if (typeof value !== "object" || value === null) return null;
  const record = value as Record<string, unknown>;
  if (
    typeof record.text !== "string" ||
    record.text.trim().length === 0 ||
    typeof record.sourceMessageId !== "string" ||
    record.sourceMessageId.length === 0 ||
    typeof record.createdAt !== "number" ||
    !Number.isFinite(record.createdAt) ||
    typeof record.bareClock !== "object" ||
    record.bareClock === null
  ) {
    return null;
  }
  const clock = record.bareClock as Record<string, unknown>;
  const hour =
    typeof clock.hour === "number" && Number.isFinite(clock.hour)
      ? Math.floor(clock.hour)
      : null;
  const minute =
    typeof clock.minute === "number" && Number.isFinite(clock.minute)
      ? Math.floor(clock.minute)
      : null;
  if (hour == null || minute == null) return null;
  if (hour < 1 || hour > 12 || minute < 0 || minute > 59) return null;
  const dayHint =
    clock.dayHint === "today" || clock.dayHint === "tomorrow"
      ? clock.dayHint
      : null;
  return {
    bareClock: { dayHint, hour, minute },
    createdAt: record.createdAt,
    sourceMessageId: record.sourceMessageId,
    text: record.text.trim(),
  };
}

/** Shared empty snapshot — stable Object.is for useSyncExternalStore. */
export const EMPTY_BESTIE_LIST_STATE: BestieListState = Object.freeze({
  items: Object.freeze([]) as unknown as BestieListItem[],
  pendingReminderConfirm: null,
  processedMessageIds: Object.freeze([]) as unknown as string[],
  version: 1,
});

export function emptyBestieListState(): BestieListState {
  return EMPTY_BESTIE_LIST_STATE;
}

export function parseBestieListItem(value: unknown): BestieListItem | null {
  if (typeof value !== "object" || value === null) return null;
  const record = value as Record<string, unknown>;
  if (
    typeof record.id !== "string" ||
    record.id.length === 0 ||
    !isKind(record.kind) ||
    typeof record.text !== "string" ||
    record.text.trim().length === 0 ||
    (record.status !== "open" && record.status !== "done") ||
    !isFiniteNonNegative(record.createdAt) ||
    !isFiniteNonNegative(record.updatedAt)
  ) {
    return null;
  }
  const dueAt =
    record.dueAt == null
      ? null
      : isFiniteNonNegative(record.dueAt)
        ? record.dueAt
        : null;
  const sourceMessageId =
    typeof record.sourceMessageId === "string" &&
    record.sourceMessageId.length > 0
      ? record.sourceMessageId
      : null;
  const starred = record.starred === true;
  const sortOrder = isFiniteNonNegative(record.sortOrder)
    ? Math.floor(record.sortOrder)
    : Math.floor(record.createdAt);
  const dayKey =
    typeof record.dayKey === "string" && /^\d{4}-\d{2}-\d{2}$/.test(record.dayKey)
      ? record.dayKey
      : record.kind === "todo"
        ? localDayKeyFromSeconds(record.createdAt)
        : null;
  const repeat =
    record.kind === "reminder" ? parseReminderRepeat(record.repeat) : null;
  return {
    createdAt: record.createdAt,
    dayKey,
    dueAt,
    id: record.id,
    kind: record.kind,
    repeat,
    sortOrder,
    sourceMessageId,
    starred,
    status: record.status,
    text: record.text.trim(),
    updatedAt: record.updatedAt,
  };
}

export function parseBestieListState(value: unknown): BestieListState | null {
  if (typeof value !== "object" || value === null) return null;
  const record = value as Record<string, unknown>;
  if (record.version !== 1 || !Array.isArray(record.items)) return null;
  const items: BestieListItem[] = [];
  for (const entry of record.items) {
    const item = parseBestieListItem(entry);
    if (item) items.push(item);
  }
  const processedMessageIds = Array.isArray(record.processedMessageIds)
    ? record.processedMessageIds.filter(
        (id): id is string => typeof id === "string" && id.length > 0,
      )
    : [];
  const pendingReminderConfirm = parsePendingReminderConfirm(
    record.pendingReminderConfirm,
  );
  return { items, pendingReminderConfirm, processedMessageIds, version: 1 };
}

function mergeBestieListStates(
  into: BestieListState,
  from: BestieListState,
): BestieListState {
  const byId = new Map(into.items.map((item) => [item.id, item]));
  for (const item of from.items) {
    const existing = byId.get(item.id);
    if (!existing || item.updatedAt >= existing.updatedAt) {
      byId.set(item.id, item);
    }
  }
  const processed = new Set([
    ...into.processedMessageIds,
    ...from.processedMessageIds,
  ]);
  const pendingReminderConfirm =
    (from.pendingReminderConfirm?.createdAt ?? 0) >=
    (into.pendingReminderConfirm?.createdAt ?? 0)
      ? from.pendingReminderConfirm
      : into.pendingReminderConfirm;
  return {
    items: [...byId.values()],
    pendingReminderConfirm,
    processedMessageIds: [...processed].slice(-200),
    version: 1,
  };
}

export function readBestieListState(scope: BestieListScope): BestieListState {
  return readOwnerScopedState({
    empty: emptyBestieListState,
    isEmpty: (state) =>
      state.items.length === 0 &&
      state.processedMessageIds.length === 0 &&
      state.pendingReminderConfirm == null,
    merge: mergeBestieListStates,
    parse: parseBestieListState,
    prefix: BESTIE_LIST_STORAGE_PREFIX,
    scope,
  });
}

export function writeBestieListState(
  scope: BestieListScope,
  state: BestieListState,
): void {
  writeOwnerScopedState(BESTIE_LIST_STORAGE_PREFIX, scope, state);
}

export function createBestieListItemId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `bestie-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

/** Due times within this many seconds count as the same reminder slot. */
export const BESTIE_LIST_DUE_DEDUPE_WINDOW_SECONDS = 120;

/**
 * Recent open items with the same kind+core-text count as duplicates even when
 * dueAt / wording differs (NL client path vs agent fence often disagree).
 */
export const BESTIE_LIST_RECENT_DEDUPE_SECONDS = 600;

function normalizeListText(text: string): string {
  return text.trim().toLowerCase();
}

/**
 * Strip clock / relative-time phrases so NL and agent fence wording compare
 * equal. Example: "8:10 to finish the nightly reports" → "finish the nightly
 * reports" (same core as "Finish the nightly reports").
 */
export function coreListTextForDedupe(text: string): string {
  let s = normalizeListText(text);
  if (!s) return "";
  s = s.replace(
    /\b(?:in|after)\s+\d+\s*(?:minutes?|mins?|m|hours?|hrs?|h|days?|d)\b/g,
    " ",
  );
  s = s.replace(/\b(?:at|by)\s+\d{1,2}(?::\d{2})?\s*(?:am|pm)?\b/g, " ");
  // Leading bare clock: "8:10 to finish…" / "8:10pm finish…"
  s = s.replace(/^\d{1,2}(?::\d{2})?\s*(?:am|pm)?\s*(?:to\s+|for\s+)?/, "");
  s = s.replace(/\b\d{1,2}:\d{2}\s*(?:am|pm)?\b/g, " ");
  s = s.replace(/\b\d{1,2}\s*(?:am|pm)\b/g, " ");
  s = s.replace(/\b(?:tomorrow|today)\b/g, " ");
  s = s.replace(/\s+/g, " ").trim();
  s = s.replace(/^(?:to|for)\s+/, "");
  return s.trim();
}

function dueAtsMatch(
  existing: number | null,
  incoming: number | null,
): boolean {
  if (existing == null && incoming == null) return true;
  if (existing == null || incoming == null) return false;
  return Math.abs(existing - incoming) <= BESTIE_LIST_DUE_DEDUPE_WINDOW_SECONDS;
}

/** True when both dues are set and within the due dedupe window. */
function dueAtsClose(
  existing: number | null,
  incoming: number | null,
): boolean {
  if (existing == null || incoming == null) return false;
  return Math.abs(existing - incoming) <= BESTIE_LIST_DUE_DEDUPE_WINDOW_SECONDS;
}

/**
 * Similar reminder wording after stripping times: exact core, containment, or
 * strong token overlap on the shorter side.
 */
export function listTextsSimilarForDedupe(a: string, b: string): boolean {
  const ca = coreListTextForDedupe(a);
  const cb = coreListTextForDedupe(b);
  if (!ca || !cb) return false;
  if (ca === cb) return true;
  if (ca.includes(cb) || cb.includes(ca)) return true;
  const tokensA = new Set(ca.split(/\s+/).filter((t) => t.length > 2));
  const tokensB = new Set(cb.split(/\s+/).filter((t) => t.length > 2));
  if (tokensA.size === 0 || tokensB.size === 0) return false;
  const [smaller, larger] =
    tokensA.size <= tokensB.size ? [tokensA, tokensB] : [tokensB, tokensA];
  let hits = 0;
  for (const token of smaller) {
    if (larger.has(token)) hits += 1;
  }
  return hits >= Math.ceil(smaller.size * 0.75) && hits >= 1;
}

/** Prefer wording that is not padded with clock phrases (higher is better). */
function listTextQualityScore(text: string): number {
  const full = normalizeListText(text);
  const core = coreListTextForDedupe(text);
  let score = 0;
  if (full === core) score += 100;
  // Shorter display text wins among equally clean cores.
  score += Math.max(0, 80 - full.length);
  return score;
}

/**
 * Find an open item that would duplicate this add (NL + agent fence case).
 * Matches on kind + core text (time phrases stripped), then due window or
 * recent creation; or close dueAt + similar wording.
 */
export function findDuplicateBestieListItem(
  state: BestieListState,
  input: BestieListAddInput,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieListItem | null {
  const needleCore = coreListTextForDedupe(input.text);
  if (!needleCore) return null;
  const incomingDue = input.kind === "reminder" ? (input.dueAt ?? null) : null;
  for (const item of state.items) {
    if (item.status !== "open") continue;
    if (item.kind !== input.kind) continue;
    const itemCore = coreListTextForDedupe(item.text);
    if (!itemCore) continue;
    const coresEqual = itemCore === needleCore;
    const similar = listTextsSimilarForDedupe(item.text, input.text);
    const recent =
      nowSeconds - item.createdAt <= BESTIE_LIST_RECENT_DEDUPE_SECONDS;

    // Same core intent: due window (incl. both null) or recent open item.
    if (coresEqual && (dueAtsMatch(item.dueAt, incomingDue) || recent)) {
      return item;
    }
    // Both have due times close together and wording is similar.
    if (dueAtsClose(item.dueAt, incomingDue) && similar) {
      return item;
    }
    // Reminder NL vs fence: one side has dueAt, texts similar, recent window.
    if (
      input.kind === "reminder" &&
      similar &&
      recent &&
      (item.dueAt != null || incomingDue != null)
    ) {
      return item;
    }
  }
  return null;
}

/** Due within this window of now counts as a "near" client NL time. */
export const BESTIE_LIST_NEAR_DUE_SECONDS = 2 * 60 * 60;

function isNearDueAt(
  dueAt: number | null,
  nowSeconds: number,
): boolean {
  if (dueAt == null) return false;
  const delta = dueAt - nowSeconds;
  return delta >= -60 && delta <= BESTIE_LIST_NEAR_DUE_SECONDS;
}

/**
 * Prefer a near dueAt (typical client NL "in 5 minutes") over a far fence
 * absolute. Fill null from the other side; otherwise keep existing.
 */
export function preferNearDueAt(
  existing: number | null,
  incoming: number | null,
  nowSeconds: number,
): number | null {
  if (existing == null) return incoming;
  if (incoming == null) return existing;
  const existingNear = isNearDueAt(existing, nowSeconds);
  const incomingNear = isNearDueAt(incoming, nowSeconds);
  if (incomingNear && !existingNear) return incoming;
  if (existingNear && !incomingNear) return existing;
  return existing;
}

/**
 * When NL and agent fence both add the same intent, keep one row. Prefer a
 * near dueAt (client NL) over a far fence epoch, fill null from incoming, and
 * keep the cleaner display text.
 */
function mergeDuplicateBestieListAdd(
  state: BestieListState,
  existing: BestieListItem,
  input: BestieListAddInput,
  nowSeconds: number,
): BestieListState {
  const incomingText = input.text.trim();
  const incomingDue = input.kind === "reminder" ? (input.dueAt ?? null) : null;
  const nextDue = preferNearDueAt(existing.dueAt, incomingDue, nowSeconds);
  const preferIncomingText =
    listTextQualityScore(incomingText) > listTextQualityScore(existing.text);
  const nextText = preferIncomingText ? incomingText : existing.text;
  if (nextDue === existing.dueAt && nextText === existing.text) {
    return state;
  }
  return {
    ...state,
    items: state.items.map((item) =>
      item.id === existing.id
        ? {
            ...item,
            dueAt: nextDue,
            text: nextText,
            updatedAt: nowSeconds,
          }
        : item,
    ),
  };
}

export function addBestieListItem(
  state: BestieListState,
  input: BestieListAddInput,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieListState {
  const text = input.text.trim();
  if (!text) return state;
  // One user ask must not create two rows (NL + agent fence use different
  // message ids, so processedMessageIds alone is not enough).
  const duplicate = findDuplicateBestieListItem(state, input, nowSeconds);
  if (duplicate) {
    return mergeDuplicateBestieListAdd(state, duplicate, input, nowSeconds);
  }
  const minOrder = state.items.reduce(
    (min, item) => Math.min(min, item.sortOrder),
    nowSeconds,
  );
  const sortOrder =
    input.sortOrder != null && Number.isFinite(input.sortOrder)
      ? Math.floor(input.sortOrder)
      : minOrder - 1;
  const dayKey =
    input.kind === "todo"
      ? input.dayKey && /^\d{4}-\d{2}-\d{2}$/.test(input.dayKey)
        ? input.dayKey
        : localDayKeyFromSeconds(nowSeconds)
      : null;
  const item: BestieListItem = {
    createdAt: nowSeconds,
    dayKey,
    dueAt: input.kind === "reminder" ? (input.dueAt ?? null) : null,
    id: createBestieListItemId(),
    kind: input.kind,
    repeat:
      input.kind === "reminder" ? (input.repeat ?? null) : null,
    sortOrder,
    sourceMessageId: input.sourceMessageId ?? null,
    starred: input.kind === "todo" ? input.starred === true : false,
    status: "open",
    text,
    updatedAt: nowSeconds,
  };
  return {
    ...state,
    items: [item, ...state.items],
  };
}

export function updateBestieListItemStatus(
  state: BestieListState,
  id: string,
  status: BestieListItem["status"],
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieListState {
  return {
    ...state,
    items: state.items.map((item) => {
      if (item.id !== id) return item;
      // Completing a recurring reminder advances to the next occurrence.
      if (
        status === "done" &&
        item.kind === "reminder" &&
        item.repeat != null &&
        item.dueAt != null
      ) {
        const from = Math.max(item.dueAt, nowSeconds);
        return {
          ...item,
          dueAt: nextBestieReminderDueAt(from, item.repeat),
          status: "open",
          updatedAt: nowSeconds,
        };
      }
      return { ...item, status, updatedAt: nowSeconds };
    }),
  };
}

export function removeBestieListItem(
  state: BestieListState,
  id: string,
): BestieListState {
  return {
    ...state,
    items: state.items.filter((item) => item.id !== id),
  };
}

export function markBestieListMessageProcessed(
  state: BestieListState,
  messageId: string,
): BestieListState {
  if (state.processedMessageIds.includes(messageId)) return state;
  const processedMessageIds = [...state.processedMessageIds, messageId];
  // Cap history so localStorage stays bounded.
  const trimmed =
    processedMessageIds.length > 200
      ? processedMessageIds.slice(processedMessageIds.length - 200)
      : processedMessageIds;
  return { ...state, processedMessageIds: trimmed };
}

export function openTodos(state: BestieListState): BestieListItem[] {
  return state.items.filter(
    (item) => item.kind === "todo" && item.status === "open",
  );
}

export function openReminders(state: BestieListState): BestieListItem[] {
  return state.items.filter(
    (item) => item.kind === "reminder" && item.status === "open",
  );
}

export function dueReminders(
  state: BestieListState,
  nowSeconds: number,
): BestieListItem[] {
  return openReminders(state).filter(
    (item) => item.dueAt != null && item.dueAt <= nowSeconds,
  );
}

export function updateBestieListItem(
  state: BestieListState,
  input: BestieListTodoUpdateInput,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieListState {
  let changed = false;
  const items = state.items.map((item) => {
    if (item.id !== input.id) return item;
    changed = true;
    const next: BestieListItem = { ...item, updatedAt: nowSeconds };
    if (input.starred != null) next.starred = input.starred;
    if (input.sortOrder != null && Number.isFinite(input.sortOrder)) {
      next.sortOrder = Math.floor(input.sortOrder);
    }
    if (input.dayKey !== undefined) {
      next.dayKey =
        input.dayKey && /^\d{4}-\d{2}-\d{2}$/.test(input.dayKey)
          ? input.dayKey
          : item.kind === "todo"
            ? localDayKeyFromSeconds(item.createdAt)
            : null;
    }
    if (input.status != null) next.status = input.status;
    if (typeof input.text === "string" && input.text.trim()) {
      next.text = input.text.trim();
    }
    return next;
  });
  return changed ? { ...state, items } : state;
}

export function toggleBestieListItemStarred(
  state: BestieListState,
  id: string,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieListState {
  const item = state.items.find((entry) => entry.id === id);
  if (!item || item.kind !== "todo") return state;
  return updateBestieListItem(
    state,
    { id, starred: !item.starred },
    nowSeconds,
  );
}

/**
 * Reorder todos after a drag. `orderedIds` is the full open-todo order within
 * the destination group (starred block or a dayKey). Updates sortOrder and
 * optionally dayKey / starred for the moved item.
 */
export function reorderBestieTodos(
  state: BestieListState,
  options: {
    /** Destination day for non-starred drops; ignored when starred. */
    dayKey?: string | null;
    orderedIds: string[];
    /** When true, items in orderedIds become starred; when false, unstarred. */
    starred: boolean;
  },
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieListState {
  const orderIndex = new Map(
    options.orderedIds.map((id, index) => [id, index]),
  );
  if (orderIndex.size === 0) return state;
  const items = state.items.map((item) => {
    if (item.kind !== "todo") return item;
    const index = orderIndex.get(item.id);
    if (index == null) return item;
    return {
      ...item,
      dayKey: options.starred
        ? item.dayKey ?? localDayKeyFromSeconds(item.createdAt)
        : options.dayKey && /^\d{4}-\d{2}-\d{2}$/.test(options.dayKey)
          ? options.dayKey
          : item.dayKey ?? localDayKeyFromSeconds(nowSeconds),
      sortOrder: index,
      starred: options.starred,
      updatedAt: nowSeconds,
    };
  });
  return { ...state, items };
}

/** Chip label: One-off vs Daily / Weekly. */
export function presentBestieReminderRepeat(
  repeat: BestieReminderRepeat | null | undefined,
): string {
  if (!repeat) return "One-off";
  if (repeat.kind === "daily") return "Daily";
  const days = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
  return `Weekly · ${days[((repeat.weekday % 7) + 7) % 7] ?? "Sun"}`;
}

/** Friendly next due for reminder rows (local date + time). */
export function formatBestieReminderDueAt(dueAt: number): string {
  const date = new Date(dueAt * 1000);
  if (!Number.isFinite(date.getTime())) return "";
  return date.toLocaleString(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

/** Next occurrence after `fromDueAt` for a repeating reminder. */
export function nextBestieReminderDueAt(
  fromDueAt: number,
  repeat: BestieReminderRepeat,
): number {
  const base = new Date(fromDueAt * 1000);
  if (repeat.kind === "daily") {
    base.setDate(base.getDate() + 1);
    return Math.floor(base.getTime() / 1000);
  }
  // Weekly: advance at least one day, then to the target weekday.
  base.setDate(base.getDate() + 1);
  while (base.getDay() !== (((repeat.weekday % 7) + 7) % 7)) {
    base.setDate(base.getDate() + 1);
  }
  return Math.floor(base.getTime() / 1000);
}

/**
 * Push dueAt forward by deltaSeconds from now (snooze). Open reminders only.
 * Returns how many items were updated.
 */
export function snoozeBestieListItems(
  state: BestieListState,
  ids: string[],
  deltaSeconds: number,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieListState {
  if (deltaSeconds <= 0 || ids.length === 0) return state;
  const idSet = new Set(ids);
  let changed = false;
  const nextDue = nowSeconds + Math.floor(deltaSeconds);
  const items = state.items.map((item) => {
    if (!idSet.has(item.id)) return item;
    if (item.kind !== "reminder" || item.status !== "open") return item;
    changed = true;
    return { ...item, dueAt: nextDue, updatedAt: nowSeconds };
  });
  return changed ? { ...state, items } : state;
}

/**
 * Dismiss due reminders: one-off → done; recurring → advance next dueAt.
 */
export function dismissBestieReminderItems(
  state: BestieListState,
  ids: string[],
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieListState {
  if (ids.length === 0) return state;
  const idSet = new Set(ids);
  let changed = false;
  const items = state.items.map((item) => {
    if (!idSet.has(item.id)) return item;
    if (item.kind !== "reminder" || item.status !== "open") return item;
    changed = true;
    if (item.repeat != null && item.dueAt != null) {
      const from = Math.max(item.dueAt, nowSeconds);
      return {
        ...item,
        dueAt: nextBestieReminderDueAt(from, item.repeat),
        updatedAt: nowSeconds,
      };
    }
    return { ...item, status: "done" as const, updatedAt: nowSeconds };
  });
  return changed ? { ...state, items } : state;
}
