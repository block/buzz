import { canonicalRelayUrl } from "@/features/agents/managedAgentRuntimeStatus";

import type {
  BestieListAddInput,
  BestieListItem,
  BestieListKind,
  BestieListScope,
  BestieListState,
} from "./bestieListTypes";

export const BESTIE_LIST_STORAGE_PREFIX = "buzz-bestie-list.v1";

export function bestieListStorageKey(scope: BestieListScope): string {
  const relay =
    canonicalRelayUrl(scope.relayUrl) ?? scope.relayUrl.trim().toLowerCase();
  return [
    BESTIE_LIST_STORAGE_PREFIX,
    relay,
    scope.ownerPubkey.toLowerCase(),
    scope.agentPubkey.toLowerCase(),
  ].join(":");
}

function isFiniteNonNegative(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

function isKind(value: unknown): value is BestieListKind {
  return value === "todo" || value === "reminder";
}

/** Shared empty snapshot — stable Object.is for useSyncExternalStore. */
export const EMPTY_BESTIE_LIST_STATE: BestieListState = Object.freeze({
  items: Object.freeze([]) as BestieListItem[],
  processedMessageIds: Object.freeze([]) as string[],
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
  return {
    createdAt: record.createdAt,
    dueAt,
    id: record.id,
    kind: record.kind,
    sourceMessageId,
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
  return { items, processedMessageIds, version: 1 };
}

export function readBestieListState(scope: BestieListScope): BestieListState {
  try {
    const raw = window.localStorage.getItem(bestieListStorageKey(scope));
    if (!raw) return emptyBestieListState();
    return parseBestieListState(JSON.parse(raw)) ?? emptyBestieListState();
  } catch {
    return emptyBestieListState();
  }
}

export function writeBestieListState(
  scope: BestieListScope,
  state: BestieListState,
): void {
  try {
    window.localStorage.setItem(
      bestieListStorageKey(scope),
      JSON.stringify(state),
    );
  } catch {
    // Quota / private mode — callers still hold in-memory state.
  }
}

export function createBestieListItemId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `bestie-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

export function addBestieListItem(
  state: BestieListState,
  input: BestieListAddInput,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieListState {
  const text = input.text.trim();
  if (!text) return state;
  const item: BestieListItem = {
    createdAt: nowSeconds,
    dueAt: input.kind === "reminder" ? (input.dueAt ?? null) : null,
    id: createBestieListItemId(),
    kind: input.kind,
    sourceMessageId: input.sourceMessageId ?? null,
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
    items: state.items.map((item) =>
      item.id === id ? { ...item, status, updatedAt: nowSeconds } : item,
    ),
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
