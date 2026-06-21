import * as React from "react";

const DONE_STORAGE_KEY = "buzz-home-feed-done.v1";
const UNREAD_STORAGE_KEY = "buzz-home-feed-unread.v1";
const MAX_ITEMS = 500;

function doneStorageKey(pubkey: string) {
  return `${DONE_STORAGE_KEY}:${pubkey}`;
}

function unreadStorageKey(pubkey: string) {
  return `${UNREAD_STORAGE_KEY}:${pubkey}`;
}

function readStoredIds(key: string): string[] {
  if (typeof window === "undefined") return [];
  const raw = window.localStorage.getItem(key);
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed)
      ? parsed
          .filter((v): v is string => typeof v === "string")
          .slice(-MAX_ITEMS)
      : [];
  } catch {
    return [];
  }
}

function writeStoredIds(key: string, ids: string[]) {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(key, JSON.stringify(ids.slice(-MAX_ITEMS)));
}

export function useFeedItemState(pubkey: string | undefined) {
  const normalizedPubkey = pubkey?.trim().toLowerCase() ?? "";
  const key = doneStorageKey(normalizedPubkey);

  const [doneIds, setDoneIds] = React.useState<string[]>(() =>
    readStoredIds(key),
  );
  const [unreadIds, setUnreadIds] = React.useState<string[]>(() =>
    readStoredIds(unreadStorageKey(normalizedPubkey)),
  );
  const [loadedPubkey, setLoadedPubkey] = React.useState(normalizedPubkey);

  React.useEffect(() => {
    setDoneIds(readStoredIds(doneStorageKey(normalizedPubkey)));
    setUnreadIds(readStoredIds(unreadStorageKey(normalizedPubkey)));
    setLoadedPubkey(normalizedPubkey);
  }, [normalizedPubkey]);

  React.useEffect(() => {
    if (loadedPubkey !== normalizedPubkey) return;
    writeStoredIds(doneStorageKey(normalizedPubkey), doneIds);
  }, [loadedPubkey, normalizedPubkey, doneIds]);

  React.useEffect(() => {
    if (loadedPubkey !== normalizedPubkey) return;
    writeStoredIds(unreadStorageKey(normalizedPubkey), unreadIds);
  }, [loadedPubkey, normalizedPubkey, unreadIds]);

  const doneSet = React.useMemo(() => new Set(doneIds), [doneIds]);
  const unreadSet = React.useMemo(() => new Set(unreadIds), [unreadIds]);

  const markDone = React.useCallback((id: string) => {
    setDoneIds((prev) => (prev.includes(id) ? prev : [...prev, id]));
    setUnreadIds((prev) => prev.filter((v) => v !== id));
  }, []);

  const undoDone = React.useCallback((id: string) => {
    setDoneIds((prev) => prev.filter((v) => v !== id));
  }, []);

  const markUnread = React.useCallback((id: string) => {
    setDoneIds((prev) => prev.filter((v) => v !== id));
    setUnreadIds((prev) => (prev.includes(id) ? prev : [...prev, id]));
  }, []);

  const undoUnread = React.useCallback((id: string) => {
    setUnreadIds((prev) => prev.filter((v) => v !== id));
  }, []);

  return { doneSet, markDone, markUnread, undoDone, undoUnread, unreadSet };
}

export type FeedItemState = ReturnType<typeof useFeedItemState>;
