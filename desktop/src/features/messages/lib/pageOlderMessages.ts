import type { QueryClient } from "@tanstack/react-query";

import { countTopLevelTimelineRows } from "@/features/messages/lib/formatTimelineMessages";
import { backfillAuxForMessages } from "@/features/messages/lib/auxBackfill";
import {
  channelMessagesKey,
  mergeTimelineHistoryMessages,
} from "@/features/messages/lib/messageQueryKeys";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";

const OLDER_MESSAGES_BATCH_SIZE = 200;

// One fetch should advance the timeline by a predictable, *visible* amount.
// Thread replies collapse into their parent and non-content events never render,
// so a single batch can add far fewer rows than that — page in more batches
// until at least this many top-level rows are added (or history runs out).
// Counting rows, not messages, keeps a reply-heavy window from feeling like the
// fetch did nothing. The cold load and scrollback share this floor so the first
// page is the same size as later ones.
export const MIN_TOP_LEVEL_ROWS_PER_FETCH = 30;

// Hard ceiling on relay pages fetched in one pass. On reply-heavy channels a
// batch yields only a few visible rows, so the row floor alone could dig through
// hundreds of messages behind one spinner. Capping per-pass keeps each fetch a
// bounded page; the scroll observer re-arms to page further while in view.
const MAX_BATCHES_PER_FETCH = 3;

export type PageOlderResult = {
  /** False once a short relay page proves history is exhausted. */
  hasOlderMessages: boolean;
};

/**
 * Page older history into the channel cache until the timeline has gained
 * {@link MIN_TOP_LEVEL_ROWS_PER_FETCH} visible rows, history runs out, or the
 * {@link MAX_BATCHES_PER_FETCH} ceiling is hit. Shared by the cold-load query
 * and the scroll-up loader so both produce the same visible page size.
 *
 * `shouldContinue` lets the caller bail mid-pass (e.g. channel switch). Returns
 * whether more history is believed to remain.
 */
export async function pageOlderMessagesUntilRowFloor(
  queryClient: QueryClient,
  channelId: string,
  shouldContinue: () => boolean,
): Promise<PageOlderResult> {
  const queryKey = channelMessagesKey(channelId);
  const baseline = queryClient.getQueryData<RelayEvent[]>(queryKey) ?? [];
  if (baseline.length === 0) {
    return { hasOlderMessages: false };
  }

  const baselineRowCount = countTopLevelTimelineRows(baseline);
  let hasOlderMessages = true;
  let batchesFetched = 0;

  // Accumulate every batch of this pass and commit to the cache once at the
  // end. Committing per batch paints the timeline in several small steps under
  // one spinner — on reply-heavy windows each 200-event batch adds only a few
  // visible rows, so the user sees the loader dribble messages in 1-5 at a
  // time. One commit = one bounded growth step.
  const fetched: RelayEvent[] = [];
  let oldestTimestamp = baseline[0].created_at;

  while (hasOlderMessages && shouldContinue()) {
    const olderMessages = await relayClient.fetchChannelHistoryBefore(
      channelId,
      oldestTimestamp,
      OLDER_MESSAGES_BATCH_SIZE,
    );
    batchesFetched += 1;

    // A full page means more likely remains; a short page is the only signal
    // of true exhaustion. An *empty* page is ambiguous (transient relay
    // pressure returns []), so don't end paging on it — let the progress guard
    // below stop this pass instead.
    if (
      olderMessages.length > 0 &&
      olderMessages.length < OLDER_MESSAGES_BATCH_SIZE
    ) {
      hasOlderMessages = false;
    }

    if (olderMessages.length > 0) {
      fetched.push(...olderMessages);
    }

    // Progress guard, not exhaustion: if the oldest timestamp didn't move back
    // (empty page, or all-duplicate), stop this pass to avoid re-fetching the
    // same `until`.
    const oldestFetched = fetched.reduce(
      (min, event) => (event.created_at < min ? event.created_at : min),
      oldestTimestamp,
    );
    if (oldestFetched >= oldestTimestamp) {
      break;
    }
    oldestTimestamp = oldestFetched;

    const rowsGained =
      countTopLevelTimelineRows(
        mergeTimelineHistoryMessages(baseline, fetched),
      ) - baselineRowCount;
    if (rowsGained >= MIN_TOP_LEVEL_ROWS_PER_FETCH) {
      break;
    }

    if (batchesFetched >= MAX_BATCHES_PER_FETCH) {
      break;
    }
  }

  if (fetched.length > 0 && shouldContinue()) {
    queryClient.setQueryData<RelayEvent[]>(queryKey, (current = []) =>
      mergeTimelineHistoryMessages(current, fetched),
    );
    void backfillAuxForMessages(queryClient, channelId, fetched);
  }

  return { hasOlderMessages };
}
