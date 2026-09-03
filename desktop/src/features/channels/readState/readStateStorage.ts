import {
  isPlainRecord,
  localIsoToUnixSeconds,
  localPublishableContextKey,
  localReadStateKey,
  localSourceCreatedAtKey,
  LOCAL_MAX_PRUNABLE_CONTEXTS,
  MSG_PREFIX,
  READ_STATE_HORIZON_SECONDS,
  THREAD_PREFIX,
  readActionRecency,
} from "@/features/channels/readState/readStateFormat";
import { setLocalStorageItemWithRecovery } from "@/shared/lib/localStorageQuota";

export type StoredReadState = {
  contexts: Map<string, number>;
  publishableContextIds: Set<string>;
  contextSourceCreatedAt: Map<string, number>;
};

function mergeLocalStorageKey(
  contexts: Map<string, number>,
  key: string,
): void {
  const raw = localStorage.getItem(key);
  if (!raw) return;

  try {
    const parsed = JSON.parse(raw);
    if (!isPlainRecord(parsed)) return;

    for (const [channelId, value] of Object.entries(parsed)) {
      const unixSeconds = localIsoToUnixSeconds(value);
      if (unixSeconds === null) continue;
      const current = contexts.get(channelId) ?? 0;
      if (unixSeconds > current) {
        contexts.set(channelId, unixSeconds);
      }
    }
  } catch (error) {
    console.debug("[ReadStateManager] storage: contexts JSON corrupt:", error);
    // Corrupt localStorage, ignore.
  }
}

function readPublishableContextIds(pubkey: string): Set<string> {
  const result = new Set<string>();
  const raw = localStorage.getItem(localPublishableContextKey(pubkey));
  if (!raw) return result;

  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return result;

    for (const value of parsed) {
      if (typeof value === "string") {
        result.add(value);
      }
    }
  } catch (error) {
    console.debug(
      "[ReadStateManager] storage: publishableContextIds JSON corrupt:",
      error,
    );
    // Corrupt localStorage, ignore.
  }

  return result;
}

function readContextSourceCreatedAt(pubkey: string): Map<string, number> {
  const result = new Map<string, number>();
  const raw = localStorage.getItem(localSourceCreatedAtKey(pubkey));
  if (!raw) return result;

  try {
    const parsed = JSON.parse(raw);
    if (!isPlainRecord(parsed)) return result;

    for (const [key, value] of Object.entries(parsed)) {
      if (typeof value === "number" && Number.isInteger(value) && value >= 0) {
        result.set(key, value);
      }
    }
  } catch (error) {
    console.debug(
      "[ReadStateManager] storage: sourceCreatedAt JSON corrupt:",
      error,
    );
    // Corrupt localStorage, ignore.
  }

  return result;
}

export function readStoredReadState(pubkey: string): StoredReadState {
  const contexts = new Map<string, number>();
  mergeLocalStorageKey(contexts, localReadStateKey(pubkey));

  return {
    contexts,
    publishableContextIds: readPublishableContextIds(pubkey),
    contextSourceCreatedAt: readContextSourceCreatedAt(pubkey),
  };
}

function isPrunableContextKey(contextId: string): boolean {
  return (
    contextId.startsWith(MSG_PREFIX) || contextId.startsWith(THREAD_PREFIX)
  );
}

/**
 * Drops msg:/thread: markers whose READ ACTION is older than the 7-day horizon,
 * then caps the survivors at LOCAL_MAX_PRUNABLE_CONTEXTS (least recently read
 * evicted first). Channel keys are never pruned — they are small, bounded by
 * membership, and losing one would resurrect the channel's unread badge.
 * Mirrors the eviction order the publish path applies in trimContextsToBudget.
 *
 * Both the horizon and the cap rank by `contextSourceCreatedAt` (see
 * `readActionRecency`), NOT by the marker value. Ranking by the marker value
 * means marking an older message read is undone by the very write that records
 * it: the new marker carries that message's old timestamp, so it sorts below
 * the cap — or below the horizon — and is dropped before it ever reaches disk.
 */
export function pruneStaleContexts(
  contexts: ReadonlyMap<string, number>,
  nowUnixSeconds: number,
  contextSourceCreatedAt?: ReadonlyMap<string, number>,
): Map<string, number> {
  const cutoff = nowUnixSeconds - READ_STATE_HORIZON_SECONDS;
  const kept = new Map<string, number>();
  const prunable: Array<{
    contextId: string;
    timestamp: number;
    recency: number;
  }> = [];

  for (const [contextId, timestamp] of contexts) {
    if (!isPrunableContextKey(contextId)) {
      kept.set(contextId, timestamp);
      continue;
    }
    const recency = readActionRecency(
      contextId,
      timestamp,
      contextSourceCreatedAt,
    );
    if (recency >= cutoff) {
      prunable.push({ contextId, timestamp, recency });
    }
  }

  if (prunable.length > LOCAL_MAX_PRUNABLE_CONTEXTS) {
    prunable.sort((a, b) => b.recency - a.recency);
    prunable.length = LOCAL_MAX_PRUNABLE_CONTEXTS;
  }
  for (const { contextId, timestamp } of prunable) {
    kept.set(contextId, timestamp);
  }
  return kept;
}

export function writeStoredReadState(
  pubkey: string,
  contexts: ReadonlyMap<string, number>,
  publishableContextIds: ReadonlySet<string>,
  contextSourceCreatedAt: ReadonlyMap<string, number>,
): void {
  const pruned = pruneStaleContexts(
    contexts,
    Math.floor(Date.now() / 1_000),
    contextSourceCreatedAt,
  );

  const state: Record<string, string> = {};
  for (const [contextId, timestamp] of pruned) {
    state[contextId] = new Date(timestamp * 1_000).toISOString();
  }

  setLocalStorageItemWithRecovery(
    localReadStateKey(pubkey),
    JSON.stringify(state),
  );
  setLocalStorageItemWithRecovery(
    localPublishableContextKey(pubkey),
    JSON.stringify([...publishableContextIds].filter((id) => pruned.has(id))),
  );

  const sourceState: Record<string, number> = {};
  for (const [contextId, createdAt] of contextSourceCreatedAt) {
    if (pruned.has(contextId)) {
      sourceState[contextId] = createdAt;
    }
  }
  setLocalStorageItemWithRecovery(
    localSourceCreatedAtKey(pubkey),
    JSON.stringify(sourceState),
  );
}
