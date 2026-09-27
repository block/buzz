import * as React from "react";

import {
  boundThreadStarStore,
  DEFAULT_THREAD_STAR_STORE,
  readThreadStarsStore,
  starredThreadEntriesFromStore,
  starredThreadIdsFromStore,
  writeThreadStarsStore,
  type StarredThreadEntry,
  type ThreadStarStore,
} from "./threadStarsStorage";

type Subscriber = () => void;

const listenersByPubkey = new Map<string, Set<Subscriber>>();
const storeByPubkey = new Map<string, ThreadStarStore>();
let storageListenerBound = false;

function ensureStorageListener(): void {
  if (storageListenerBound || typeof window === "undefined") return;
  storageListenerBound = true;
  window.addEventListener("storage", (event) => {
    if (!event.key?.startsWith("buzz-thread-stars.v1:")) return;
    const pubkey = event.key.slice("buzz-thread-stars.v1:".length);
    if (!pubkey) return;
    storeByPubkey.delete(pubkey);
    notifyPubkey(pubkey);
  });
}

function notifyPubkey(pubkey: string): void {
  const listeners = listenersByPubkey.get(pubkey);
  if (!listeners) return;
  for (const listener of listeners) listener();
}

function getStore(pubkey: string): ThreadStarStore {
  const cached = storeByPubkey.get(pubkey);
  if (cached) return cached;
  const store = readThreadStarsStore(pubkey);
  storeByPubkey.set(pubkey, store);
  return store;
}

function setStore(pubkey: string, store: ThreadStarStore): boolean {
  const bounded = boundThreadStarStore(store);
  if (!writeThreadStarsStore(pubkey, bounded)) return false;
  storeByPubkey.set(pubkey, bounded);
  notifyPubkey(pubkey);
  return true;
}

function subscribe(pubkey: string, listener: Subscriber): () => void {
  ensureStorageListener();
  let listeners = listenersByPubkey.get(pubkey);
  if (!listeners) {
    listeners = new Set();
    listenersByPubkey.set(pubkey, listeners);
  }
  listeners.add(listener);
  return () => {
    listeners?.delete(listener);
    if (listeners && listeners.size === 0) {
      listenersByPubkey.delete(pubkey);
    }
  };
}

export type StarThreadInput = {
  rootId: string;
  channelId: string;
  title: string;
  channelName: string;
};

export function useThreadStars(pubkey: string | undefined): {
  starredThreadIds: ReadonlySet<string>;
  starredThreads: readonly StarredThreadEntry[];
  isThreadStarred: (rootId: string) => boolean;
  starThread: (input: StarThreadInput) => void;
  unstarThread: (rootId: string) => void;
  toggleThreadStar: (input: StarThreadInput) => void;
} {
  const store = React.useSyncExternalStore(
    React.useCallback(
      (onStoreChange) => {
        if (!pubkey) return () => {};
        return subscribe(pubkey, onStoreChange);
      },
      [pubkey],
    ),
    React.useCallback(
      () => (pubkey ? getStore(pubkey) : DEFAULT_THREAD_STAR_STORE),
      [pubkey],
    ),
    () => DEFAULT_THREAD_STAR_STORE,
  );

  const starredThreadIds = React.useMemo(
    () => starredThreadIdsFromStore(store),
    [store],
  );
  const starredThreads = React.useMemo(
    () => starredThreadEntriesFromStore(store),
    [store],
  );

  const isThreadStarred = React.useCallback(
    (rootId: string) => starredThreadIds.has(rootId),
    [starredThreadIds],
  );

  const starThread = React.useCallback(
    (input: StarThreadInput) => {
      if (!pubkey) return;
      const rootId = input.rootId.trim();
      const channelId = input.channelId.trim();
      if (!rootId || !channelId) return;
      const entry: StarredThreadEntry = {
        rootId,
        channelId,
        title: input.title.trim() || "Thread",
        channelName: input.channelName.trim() || "channel",
        starredAt: Math.floor(Date.now() / 1000),
      };
      const prev = getStore(pubkey);
      setStore(
        pubkey,
        boundThreadStarStore(
          {
            version: 1,
            threads: { ...prev.threads, [rootId]: entry },
          },
          rootId,
        ),
      );
    },
    [pubkey],
  );

  const unstarThread = React.useCallback(
    (rootId: string) => {
      if (!pubkey) return;
      const trimmed = rootId.trim();
      if (!trimmed) return;
      const prev = getStore(pubkey);
      if (!(trimmed in prev.threads)) return;
      const nextThreads = { ...prev.threads };
      delete nextThreads[trimmed];
      setStore(pubkey, { version: 1, threads: nextThreads });
    },
    [pubkey],
  );

  const toggleThreadStar = React.useCallback(
    (input: StarThreadInput) => {
      if (isThreadStarred(input.rootId)) {
        unstarThread(input.rootId);
      } else {
        starThread(input);
      }
    },
    [isThreadStarred, starThread, unstarThread],
  );

  return {
    starredThreadIds,
    starredThreads,
    isThreadStarred,
    starThread,
    unstarThread,
    toggleThreadStar,
  };
}

/** Test helper: drop in-memory cache so the next read hits storage. */
export function __resetThreadStarsCacheForTests(): void {
  storeByPubkey.clear();
}
