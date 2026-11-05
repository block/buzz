import type { ThreadTitleRecord } from "@/shared/api/tauriThreadTitles";

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
