import {
  isPlainRecord,
  localIsoToUnixSeconds,
  localPublishableContextKey,
  localReadStateKey,
  localSourceCreatedAtKey,
} from "@/features/channels/readState/readStateFormat";

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

export function writeStoredReadState(
  pubkey: string,
  contexts: ReadonlyMap<string, number>,
  publishableContextIds: ReadonlySet<string>,
  contextSourceCreatedAt: ReadonlyMap<string, number>,
): void {
  const state: Record<string, string> = {};
  for (const [contextId, timestamp] of contexts) {
    state[contextId] = new Date(timestamp * 1_000).toISOString();
  }

  localStorage.setItem(localReadStateKey(pubkey), JSON.stringify(state));
  localStorage.setItem(
    localPublishableContextKey(pubkey),
    JSON.stringify([...publishableContextIds]),
  );

  const sourceState: Record<string, number> = {};
  for (const [contextId, createdAt] of contextSourceCreatedAt) {
    sourceState[contextId] = createdAt;
  }
  localStorage.setItem(
    localSourceCreatedAtKey(pubkey),
    JSON.stringify(sourceState),
  );
}
