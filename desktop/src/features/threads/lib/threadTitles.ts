import type {
  ThreadTitle,
  ThreadTitleRecord,
} from "@/shared/api/tauriThreadTitles";

/** Same bound the relay enforces on the NIP-AR `title` tag. */
export const MAX_THREAD_TITLE_BYTES = 512;

export const THREAD_TITLES_QUERY_KEY = ["thread-titles"] as const;

/**
 * One root's title. Nested under {@link THREAD_TITLES_QUERY_KEY} so every
 * title invalidation also refreshes the per-root lookups.
 */
export function threadTitleRootQueryKey(rootId: string) {
  return [...THREAD_TITLES_QUERY_KEY, "root", rootId.toLowerCase()] as const;
}

const ROOT_ID_PATTERN = /^[0-9a-f]{64}$/;

type TitleWaiter = {
  resolve: (title: ThreadTitleRecord | null) => void;
  reject: (error: unknown) => void;
};

/**
 * Coalesces per-root title lookups made in the same tick into one fetch, so a
 * timeline full of thread roots costs one lookup rather than one per root.
 * Ids that can't be event ids (e.g. a DM conversation id) resolve to `null`
 * without a fetch.
 */
export function createThreadTitleBatcher(
  fetchTitles: (rootIds: string[]) => Promise<readonly ThreadTitleRecord[]>,
  schedule: (flush: () => void) => void = (flush) => {
    globalThis.setTimeout(flush, 0);
  },
): (rootId: string) => Promise<ThreadTitleRecord | null> {
  let pending = new Map<string, TitleWaiter[]>();

  const flush = async () => {
    const batch = pending;
    pending = new Map();
    try {
      const titles = await fetchTitles([...batch.keys()]);
      const byRoot = new Map(
        titles.map((entry) => [entry.rootId.toLowerCase(), entry]),
      );
      for (const [root, waiters] of batch) {
        for (const waiter of waiters) {
          waiter.resolve(byRoot.get(root) ?? null);
        }
      }
    } catch (error) {
      for (const waiters of batch.values()) {
        for (const waiter of waiters) {
          waiter.reject(error);
        }
      }
    }
  };

  return (rootId) => {
    const root = rootId.toLowerCase();
    if (!ROOT_ID_PATTERN.test(root)) {
      return Promise.resolve(null);
    }
    return new Promise((resolve, reject) => {
      const waiters = pending.get(root);
      if (waiters) {
        waiters.push({ resolve, reject });
        return;
      }
      if (pending.size === 0) {
        schedule(() => void flush());
      }
      pending.set(root, [{ resolve, reject }]);
    });
  };
}

/** The shared title for a thread, if one is set. */
export function findThreadTitle(
  titles: readonly ThreadTitle[] | undefined,
  channelId: string | null,
  rootId: string | null,
): ThreadTitle | null {
  if (!titles || !channelId || !rootId) {
    return null;
  }
  const root = rootId.toLowerCase();
  return (
    titles.find(
      (entry) => entry.channelId === channelId && entry.rootId === root,
    ) ?? null
  );
}

/** User-facing message for a failed title save. */
export function describeTitleSaveError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  if (message.startsWith("conflict:")) {
    return "Someone else changed this thread's title while you were editing. Your change wasn't saved.";
  }
  return `Couldn't save the thread title: ${message}`;
}

export type TitleDraftResult =
  | { kind: "set"; title: string }
  | { kind: "clear" }
  | { kind: "unchanged" }
  | { kind: "too-long" };

/** Interpret an edited title: blank clears it, an unchanged value writes nothing. */
export function interpretTitleDraft(
  draft: string,
  current: string | null,
): TitleDraftResult {
  const title = draft.trim();
  if (title === (current ?? "")) {
    return { kind: "unchanged" };
  }
  if (title.length === 0) {
    return { kind: "clear" };
  }
  if (new TextEncoder().encode(title).length > MAX_THREAD_TITLE_BYTES) {
    return { kind: "too-long" };
  }
  return { kind: "set", title };
}

/** Optimistically apply a write to the cached list so the UI updates before the refetch. */
export function applyTitleWrite(
  titles: readonly ThreadTitle[] | undefined,
  write: {
    channelId: string;
    rootId: string;
    title: string | null;
    /** Stored revision once known; until then the previous one is kept. */
    revision?: string | null;
  },
  nowSeconds: number,
): ThreadTitle[] {
  const root = write.rootId.toLowerCase();
  const rest = (titles ?? []).filter(
    (entry) => !(entry.channelId === write.channelId && entry.rootId === root),
  );
  if (write.title === null) {
    return rest;
  }
  const previous = findThreadTitle(titles, write.channelId, root);
  return [
    {
      author: previous?.author ?? "",
      channelId: write.channelId,
      lastActivityAt: Math.max(previous?.lastActivityAt ?? 0, nowSeconds),
      revision: write.revision ?? previous?.revision ?? "",
      rootId: root,
      title: write.title,
      updatedAt: nowSeconds,
    },
    ...rest,
  ];
}

/** Optimistically apply a write to one root's cached title. */
export function applyRootTitleWrite(
  previous: ThreadTitleRecord | null | undefined,
  write: {
    channelId: string;
    rootId: string;
    title: string | null;
    /** Stored revision once known; until then the previous one is kept. */
    revision?: string | null;
  },
  nowSeconds: number,
): ThreadTitleRecord | null {
  if (write.title === null) {
    return null;
  }
  return {
    author: previous?.author ?? "",
    channelId: write.channelId,
    revision: write.revision ?? previous?.revision ?? "",
    rootId: write.rootId.toLowerCase(),
    title: write.title,
    updatedAt: nowSeconds,
  };
}

/**
 * Unread reply counts per thread root, for the titled threads only. Replies
 * are counted once each, so a reply listed twice (live activity plus a
 * reopened Inbox row) can't inflate a count.
 */
export function countUnreadByTitledRoot(
  titles: readonly ThreadTitle[] | undefined,
  unreadReplies: ReadonlyArray<{ id: string; rootId: string | null }>,
): Map<string, number> {
  const titledRoots = new Set((titles ?? []).map((entry) => entry.rootId));
  const seen = new Set<string>();
  const counts = new Map<string, number>();
  for (const reply of unreadReplies) {
    const root = reply.rootId?.toLowerCase();
    if (!root || !titledRoots.has(root) || seen.has(reply.id)) {
      continue;
    }
    seen.add(reply.id);
    counts.set(root, (counts.get(root) ?? 0) + 1);
  }
  return counts;
}

/** Agent work across the threads the Threads view lists. */
export type WorkingTitledThreads = {
  /** Start of the longest-running turn, on the desktop clock. */
  anchorAt: number;
  threadCount: number;
};

/**
 * Rolls active thread turns up for the sidebar's Threads entry. Untitled
 * threads are left out: the view doesn't list them, so counting them would
 * light the entry with no row to find.
 */
export function summarizeWorkingTitledThreads(
  titles: readonly ThreadTitle[] | undefined,
  workingThreads: ReadonlyMap<string, { anchorAt: number }>,
): WorkingTitledThreads | null {
  if (!titles || workingThreads.size === 0) return null;
  const titledRoots = new Set(titles.map((entry) => entry.rootId));
  let anchorAt = Number.POSITIVE_INFINITY;
  let threadCount = 0;
  for (const [rootId, summary] of workingThreads) {
    if (!titledRoots.has(rootId.toLowerCase())) continue;
    threadCount += 1;
    anchorAt = Math.min(anchorAt, summary.anchorAt);
  }
  return threadCount === 0 ? null : { anchorAt, threadCount };
}

/** The relay rejects a REQ naming more than 128 `#h` values. */
export const MAX_LIVE_SYNC_CHANNELS_PER_FILTER = 128;

/**
 * Deduplicated, sorted channel batches for the title live sync. Sorting makes
 * the batching stable so an unrelated re-render doesn't resubscribe.
 */
export function batchTitleSyncChannels(
  channelIds: readonly string[],
  batchSize = MAX_LIVE_SYNC_CHANNELS_PER_FILTER,
): string[][] {
  const unique = Array.from(new Set(channelIds.filter(Boolean))).sort();
  const batches: string[][] = [];
  for (let i = 0; i < unique.length; i += batchSize) {
    batches.push(unique.slice(i, i + batchSize));
  }
  return batches;
}
