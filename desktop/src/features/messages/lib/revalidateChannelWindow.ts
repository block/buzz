import {
  appendOlderChannelWindow,
  compareRelayOrder,
  emptyChannelWindowStore,
  replaceNewestChannelWindow,
  type ChannelWindowCursor,
  type ChannelWindowPage,
  type ChannelWindowStore,
} from "./channelWindowStore";

/** Stage a refreshed head without evicting the reading window. Reuse history
 * only at an exact cursor join; otherwise follow server cursors until the new
 * chain covers the old window. Failure leaves the published store untouched.
 */
export async function revalidateChannelWindow<T>({
  head,
  retained,
  readCurrent,
  fetchPage,
  signal,
  publish,
}: {
  head: ChannelWindowPage;
  retained: ChannelWindowStore;
  readCurrent: () => ChannelWindowStore;
  fetchPage: (
    cursor: ChannelWindowCursor,
    limitRows: number,
  ) => Promise<ChannelWindowPage>;
  signal: AbortSignal;
  /** Called synchronously after the final readCurrent check; no await gap may
   * let a concurrent older page extend the reading window before publication. */
  publish: (pages: ChannelWindowPage[]) => T;
}): Promise<T> {
  let staged = replaceNewestChannelWindow(emptyChannelWindowStore(), head);
  // Bound reconnect work, including a reader extending history concurrently.
  // A busy channel can retry later; silently truncating the reader is not an
  // acceptable fallback when the new head is too far away.
  const budget = retained.pages.length + 3;
  let verifiedJoin = false;
  for (let requests = 0; ; requests++) {
    signal.throwIfAborted();
    const current = readCurrent();
    const oldTail = current.pages.at(-1);
    const tail = staged.pages.at(-1);
    if (!tail) throw new Error("History refresh has no staged head.");
    if (!tail.hasMore || !oldTail) return publish(staged.pages);
    const cursor = tail.nextCursor;
    if (!cursor) throw new Error("History refresh is missing its next cursor.");
    const join = current.pages.findIndex(
      (page) =>
        page.startCursor?.createdAt === cursor.createdAt &&
        page.startCursor.eventId === cursor.eventId,
    );
    if (join >= 0 && verifiedJoin) {
      for (const page of current.pages.slice(join))
        staged = appendOlderChannelWindow(staged, page);
      return publish(staged.pages);
    }
    const oldest = oldTail.rows.at(-1)?.event;
    if (
      oldest &&
      compareRelayOrder(
        { created_at: cursor.createdAt, id: cursor.eventId } as typeof oldest,
        oldest,
      ) >= 0
    )
      return publish(staged.pages);
    if (requests >= budget)
      throw new Error(
        "History refresh exceeded its reading-window budget; retaining the current timeline.",
      );
    let limitRows = 50;
    // Ask the relay for a short bridge to an old page boundary instead of
    // re-fetching all retained history when a one-row head change misaligns
    // every 50-row page. This is only a request-size hint: echoed bounds remain
    // authoritative, including when rows were deleted or reconstruction skips.
    for (const page of current.pages) {
      const index = page.rows.findIndex(
        (row) =>
          row.event.id === cursor.eventId &&
          row.event.created_at === cursor.createdAt,
      );
      if (index >= 0 && index + 1 < page.rows.length) {
        limitRows = page.rows.length - index - 1;
        break;
      }
    }
    // Re-read one page AFTER the first exact join before adopting deeper
    // immutable history. A new dense-second row can sort immediately after the
    // old boundary; the short bridge ending at that boundary cannot see it.
    // Only the first join is verified; deeper boundaries are trusted under
    // NIP-CW's immutable-history contract, not arbitrary backfill repair.
    if (join >= 0) verifiedJoin = true;
    staged = appendOlderChannelWindow(
      staged,
      await fetchPage(cursor, limitRows),
    );
  }
}
