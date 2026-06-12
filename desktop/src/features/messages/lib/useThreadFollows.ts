import * as React from "react";

const STORAGE_KEY_PREFIX = "buzz-thread-follows.v1";
const MAX_ENTRIES = 500;

type ThreadFollowEntry = {
  rootId: string;
  followedAt: number;
};

function storageKey(pubkey: string): string {
  return `${STORAGE_KEY_PREFIX}:${pubkey}`;
}

function readFromStorage(pubkey: string): ThreadFollowEntry[] {
  try {
    const raw = window.localStorage.getItem(storageKey(pubkey));
    if (!raw) {
      return [];
    }
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return [];
    }
    return parsed.filter(
      (entry): entry is ThreadFollowEntry =>
        typeof entry === "object" &&
        entry !== null &&
        typeof entry.rootId === "string" &&
        typeof entry.followedAt === "number",
    );
  } catch {
    return [];
  }
}

function writeToStorage(pubkey: string, entries: ThreadFollowEntry[]): boolean {
  try {
    window.localStorage.setItem(storageKey(pubkey), JSON.stringify(entries));
    return true;
  } catch {
    return false;
  }
}

function capEntries(entries: ThreadFollowEntry[]): ThreadFollowEntry[] {
  if (entries.length <= MAX_ENTRIES) {
    return entries;
  }
  return entries
    .slice()
    .sort((a, b) => a.followedAt - b.followedAt)
    .slice(entries.length - MAX_ENTRIES);
}

export function useThreadFollows(pubkey: string | undefined): {
  followedRootIds: ReadonlySet<string>;
  isFollowing: (rootId: string) => boolean;
  followThread: (rootId: string) => void;
  unfollowThread: (rootId: string) => void;
} {
  const [entries, setEntries] = React.useState<ThreadFollowEntry[]>(() => {
    if (!pubkey) {
      return [];
    }
    return readFromStorage(pubkey);
  });

  React.useEffect(() => {
    if (!pubkey) {
      setEntries([]);
      return;
    }
    setEntries(readFromStorage(pubkey));
  }, [pubkey]);

  React.useEffect(() => {
    if (!pubkey) {
      return;
    }
    const key = storageKey(pubkey);
    const handler = (e: StorageEvent) => {
      if (e.key !== key) {
        return;
      }
      setEntries(readFromStorage(pubkey));
    };
    window.addEventListener("storage", handler);
    return () => {
      window.removeEventListener("storage", handler);
    };
  }, [pubkey]);

  const followedRootIds = React.useMemo<ReadonlySet<string>>(
    () => new Set(entries.map((e) => e.rootId)),
    [entries],
  );

  const isFollowing = React.useCallback(
    (rootId: string) => followedRootIds.has(rootId),
    [followedRootIds],
  );

  const followThread = React.useCallback(
    (rootId: string) => {
      if (!pubkey) {
        return;
      }
      setEntries((prev) => {
        if (prev.some((e) => e.rootId === rootId)) {
          return prev;
        }
        const next = capEntries([...prev, { rootId, followedAt: Date.now() }]);
        if (!writeToStorage(pubkey, next)) {
          return prev;
        }
        return next;
      });
    },
    [pubkey],
  );

  const unfollowThread = React.useCallback(
    (rootId: string) => {
      if (!pubkey) {
        return;
      }
      setEntries((prev) => {
        const next = prev.filter((e) => e.rootId !== rootId);
        if (!writeToStorage(pubkey, next)) {
          return prev;
        }
        return next;
      });
    },
    [pubkey],
  );

  return { followedRootIds, isFollowing, followThread, unfollowThread };
}
