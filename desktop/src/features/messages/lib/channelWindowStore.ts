import type { RelayEvent } from "@/shared/api/types";

export type ChannelWindowCursor = { createdAt: number; eventId: string };
export type ChannelWindowThreadSummary = {
  replyCount: number;
  descendantCount: number;
  lastReplyAt: number | null;
  participantPubkeys: string[];
};
export type ChannelWindowRow = {
  event: RelayEvent;
  thread: ChannelWindowThreadSummary | null;
};
export type LiveThreadSummary = {
  summary: ChannelWindowThreadSummary;
  /** `created_at` of the relay 39005 that carried it — newest wins per root. */
  createdAt: number;
};
export type ChannelWindowPage = {
  startCursor: ChannelWindowCursor | null;
  rows: ChannelWindowRow[];
  aux: RelayEvent[];
  nextCursor: ChannelWindowCursor | null;
  hasMore: boolean;
};
export type ChannelWindowStore = {
  pages: ChannelWindowPage[];
  /** Top-level live events not represented in an authoritative relay page. */
  liveOverlay: RelayEvent[];
  /** Live structural events retained independently from frozen page closure. */
  liveAux: RelayEvent[];
  /**
   * Relay-pushed 39005 summaries (keyed by thread root id) not yet superseded
   * by a page. A page response supersedes them for the roots it re-delivers.
   */
  liveSummaries: Record<string, LiveThreadSummary>;
};

export const emptyChannelWindowStore = (): ChannelWindowStore => ({
  pages: [],
  liveOverlay: [],
  liveAux: [],
  liveSummaries: {},
});

function cursorsEqual(
  left: ChannelWindowCursor | null,
  right: ChannelWindowCursor | null,
) {
  return (
    left === right ||
    (left !== null &&
      right !== null &&
      left.createdAt === right.createdAt &&
      left.eventId === right.eventId)
  );
}

/** Relay order: newest timestamp first, then ascending id within a second. */
function compareRelayOrder(left: RelayEvent, right: RelayEvent) {
  return left.created_at !== right.created_at
    ? right.created_at - left.created_at
    : left.id < right.id
      ? -1
      : left.id > right.id
        ? 1
        : 0;
}

function isStrictlyOlder(event: RelayEvent, cursor: ChannelWindowCursor) {
  return (
    event.created_at < cursor.createdAt ||
    (event.created_at === cursor.createdAt && event.id > cursor.eventId)
  );
}

function assertValidPage(page: ChannelWindowPage) {
  if (page.hasMore !== (page.nextCursor !== null)) {
    throw new Error("Channel window hasMore and nextCursor disagree.");
  }
  const seen = new Set<string>();
  for (let index = 0; index < page.rows.length; index += 1) {
    const event = page.rows[index].event;
    if (seen.has(event.id))
      throw new Error(`Duplicate channel row ${event.id}.`);
    seen.add(event.id);
    if (page.startCursor && !isStrictlyOlder(event, page.startCursor)) {
      throw new Error(
        `Channel row ${event.id} is outside its cursor interval.`,
      );
    }
    const previous = page.rows[index - 1]?.event;
    if (previous && compareRelayOrder(previous, event) > 0) {
      throw new Error("Channel window rows are not in relay order.");
    }
  }
}

/**
 * Replace the authoritative chain at page zero. Its end cursor may move, so
 * retaining old tail pages would claim a cursor chain they were not fetched on.
 */
export function replaceNewestChannelWindow(
  current: ChannelWindowStore,
  page: ChannelWindowPage,
): ChannelWindowStore {
  if (page.startCursor !== null) {
    throw new Error("Newest channel page must have a null start cursor.");
  }
  assertValidPage(page);
  const ids = new Set(page.rows.map((row) => row.event.id));
  const auxIds = new Set(page.aux.map((event) => event.id));
  return {
    pages: [page],
    liveOverlay: current.liveOverlay.filter((event) => !ids.has(event.id)),
    liveAux: current.liveAux.filter((event) => !auxIds.has(event.id)),
    // A head refetch is the authoritative resync moment (subscribe/reconnect
    // both funnel here): every retained page is replaced, so live summaries
    // pinned to the old snapshot are cleared rather than diffed.
    liveSummaries: {},
  };
}

/** Append only a response that continues the retained echoed cursor chain. */
export function appendOlderChannelWindow(
  current: ChannelWindowStore,
  page: ChannelWindowPage,
): ChannelWindowStore {
  assertValidPage(page);
  const tail = current.pages[current.pages.length - 1];
  if (!tail) throw new Error("Load the newest channel page first.");
  if (!tail.hasMore || !tail.nextCursor) {
    throw new Error("The channel window is already complete.");
  }
  if (!cursorsEqual(page.startCursor, tail.nextCursor)) {
    throw new Error(
      "Channel page does not continue the retained cursor chain.",
    );
  }
  const ids = new Set(
    current.pages.flatMap((retained) =>
      retained.rows.map((row) => row.event.id),
    ),
  );
  for (const row of page.rows) {
    if (ids.has(row.event.id)) {
      throw new Error(`Channel row ${row.event.id} overlaps a retained page.`);
    }
  }
  const pageIds = new Set(page.rows.map((row) => row.event.id));
  return {
    ...current,
    pages: [...current.pages, page],
    liveOverlay: current.liveOverlay.filter((event) => !pageIds.has(event.id)),
  };
}

/**
 * Record a relay-pushed live `39005` summary. Newest `created_at` wins per
 * root: the relay pushes a full recount on every thread mutation, so the
 * latest push is authoritative for that root — including counting *down*
 * after a delete. Retained across scrollback pages (a racing push can be
 * fresher than a just-fetched page summary) and cleared only by the head
 * refetch in `replaceNewestChannelWindow`.
 */
export function mergeLiveThreadSummary(
  current: ChannelWindowStore,
  rootId: string,
  incoming: LiveThreadSummary,
): ChannelWindowStore {
  const existing = current.liveSummaries[rootId];
  if (existing && existing.createdAt >= incoming.createdAt) return current;
  return {
    ...current,
    liveSummaries: { ...current.liveSummaries, [rootId]: incoming },
  };
}

/**
 * Merge a live top-level event without mutating authoritative page boundaries.
 * Events below the oldest loaded boundary wait for ordinary relay pagination.
 */
export function mergeLiveChannelWindowEvent(
  current: ChannelWindowStore,
  event: RelayEvent,
  isTimelineRow = true,
): ChannelWindowStore {
  if (!isTimelineRow) {
    if (
      current.liveAux.some((candidate) => candidate.id === event.id) ||
      current.pages.some((page) =>
        page.aux.some((candidate) => candidate.id === event.id),
      )
    ) {
      return current;
    }
    return { ...current, liveAux: [...current.liveAux, event] };
  }
  if (
    current.pages.some((page) =>
      page.rows.some((row) => row.event.id === event.id),
    )
  ) {
    return current;
  }
  const oldestPage = current.pages[current.pages.length - 1];
  const oldest = oldestPage?.rows[oldestPage.rows.length - 1]?.event;
  if (oldest && compareRelayOrder(event, oldest) >= 0) return current;
  return {
    ...current,
    liveOverlay: current.liveOverlay
      .filter((candidate) => candidate.id !== event.id)
      .concat(event)
      .sort(compareRelayOrder),
  };
}

/** Raw events in the chronological order expected by the existing renderer. */
export function flattenChannelWindowEvents(store: ChannelWindowStore) {
  const byId = new Map<string, RelayEvent>();
  for (const page of store.pages) {
    for (const row of page.rows) byId.set(row.event.id, row.event);
    for (const event of page.aux) byId.set(event.id, event);
  }
  for (const event of store.liveOverlay) byId.set(event.id, event);
  for (const event of store.liveAux) byId.set(event.id, event);
  return [...byId.values()].sort((left, right) =>
    compareRelayOrder(right, left),
  );
}

/**
 * Whether older history remains beyond the retained pages. The authoritative
 * signal for paging: the tail page's `hasMore` (kept in lockstep with its
 * `nextCursor` by `assertValidPage`). Empty pages mean no window has loaded
 * yet — there is no cursor to page with, so report false; the first
 * `replaceNewestChannelWindow` makes the signal authoritative.
 */
export function channelWindowHasMore(store: ChannelWindowStore) {
  const tail = store.pages[store.pages.length - 1];
  return tail?.hasMore ?? false;
}

/**
 * Per-root thread summaries for badge rendering: authoritative page summaries
 * overlaid with any fresher relay-pushed live summaries. The live overlay also
 * covers roots that reached the screen outside a page (liveOverlay rows,
 * refetch-reconciled rows), which have no page summary at all.
 */
export function channelWindowThreadSummaries(store: ChannelWindowStore) {
  const summaries = new Map(
    store.pages.flatMap((page) =>
      page.rows.flatMap((row) =>
        row.thread ? ([[row.event.id, row.thread]] as const) : [],
      ),
    ),
  );
  for (const [rootId, live] of Object.entries(store.liveSummaries)) {
    summaries.set(rootId, live.summary);
  }
  return summaries;
}
