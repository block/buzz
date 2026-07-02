import type { QueryClient } from "@tanstack/react-query";

import { countTopLevelTimelineRows } from "@/features/messages/lib/formatTimelineMessages";
import { backfillAuxForMessages } from "@/features/messages/lib/auxBackfill";
import {
  channelMessagesKey,
  mergeTimelineHistoryMessages,
} from "@/features/messages/lib/messageQueryKeys";
import { relayClient } from "@/shared/api/relayClient";
import { getChannelMessagesBefore } from "@/shared/api/tauri";
import type { ChannelPageCursor, RelayEvent } from "@/shared/api/types";

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

// One paging pass per channel at a time: the background cold-load top-up and
// a scroll-up fetch share the running pass instead of overlapping REQs.
const inFlightPasses = new Map<string, Promise<PageOlderResult>>();

/**
 * Seed the bridge keyset cursor for the dense-second escape hatch.
 *
 * The relay orders `created_at DESC, id ASC` and advances past a second denser
 * than one WS page via `id > before_id`. So the cursor must point at the
 * *furthest* relay-order position already known at the stalled second — the
 * **max** event id among all cached/fetched events at `created_at === second`.
 * Seeding from the min id (e.g. `baseline[0].id`) would re-request rows already
 * held; seeding from the max id asks the relay for the strictly-unreached tail.
 */
function maxEventIdAtSecond(
  events: RelayEvent[],
  second: number,
): string | null {
  let maxId: string | null = null;
  for (const event of events) {
    if (event.created_at !== second) {
      continue;
    }
    if (maxId === null || event.id > maxId) {
      maxId = event.id;
    }
  }
  return maxId;
}

/**
 * Dense-second escape hatch: drain older history via the bridge composite
 * keyset once the WS `until` cursor has stalled on a second denser than one
 * page. Seeds from the max event id at `boundarySecond` (the furthest
 * relay-order position already held) and pages `(created_at, event_id)`,
 * clearing the *entire* boundary second first (the wall must be broken in the
 * pass that detects it), then honoring the shared per-pass row-floor / batch
 * budget for any older history. Stops on a short page (history exhausted).
 * Appends fetched events to `fetched` in place; returns whether more history is
 * believed to remain (`false` only on a short page).
 */
async function drainOlderViaKeyset(args: {
  channelId: string;
  boundarySecond: number;
  baseline: RelayEvent[];
  fetched: RelayEvent[];
  baselineRowCount: number;
  batchesFetched: number;
  shouldContinue: () => boolean;
}): Promise<boolean> {
  const {
    channelId,
    boundarySecond,
    baseline,
    fetched,
    baselineRowCount,
    shouldContinue,
  } = args;

  const seedId = maxEventIdAtSecond([...baseline, ...fetched], boundarySecond);
  if (seedId === null) {
    // Nothing known at the boundary second to key off — shouldn't happen once
    // stalled, but bail rather than fabricate a cursor.
    return true;
  }

  let cursor: ChannelPageCursor | null = {
    createdAt: boundarySecond,
    eventId: seedId,
  };
  let batchesFetched = args.batchesFetched;
  let hasOlderMessages = true;

  while (cursor !== null && shouldContinue()) {
    const page = await getChannelMessagesBefore(
      channelId,
      cursor,
      OLDER_MESSAGES_BATCH_SIZE,
    );
    batchesFetched += 1;

    if (page.events.length > 0) {
      fetched.push(...page.events);
    }

    // A null next cursor means the relay returned a short page — history is
    // exhausted. A full page yields a next cursor to continue from.
    if (page.nextCursor === null) {
      hasOlderMessages = false;
      break;
    }
    cursor = page.nextCursor;

    // Fully clear the dense second before honoring the per-pass budget. The
    // wall is a single `created_at` second denser than one page; yielding while
    // the cursor is still *inside* that second would strand its tail behind the
    // same stall, and the scroll sentinel may not re-arm to summon us again —
    // the wall must be cleared in the pass that detects it. Once the cursor has
    // advanced to an older second, ordinary older history resumes and the row
    // floor / batch budget bound the pass as usual.
    if (cursor.createdAt >= boundarySecond) {
      continue;
    }

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

  return hasOlderMessages;
}

/**
 * Page older history into the channel cache until the timeline has gained
 * {@link MIN_TOP_LEVEL_ROWS_PER_FETCH} visible rows, history runs out, or the
 * {@link MAX_BATCHES_PER_FETCH} ceiling is hit. Shared by the cold-load query
 * and the scroll-up loader so both produce the same visible page size.
 *
 * `shouldContinue` lets the caller bail mid-pass (e.g. channel switch). Returns
 * whether more history is believed to remain.
 */
export function pageOlderMessagesUntilRowFloor(
  queryClient: QueryClient,
  channelId: string,
  shouldContinue: () => boolean,
): Promise<PageOlderResult> {
  const inFlight = inFlightPasses.get(channelId);
  if (inFlight) {
    return inFlight;
  }

  const pass = runPageOlderPass(queryClient, channelId, shouldContinue).finally(
    () => {
      inFlightPasses.delete(channelId);
    },
  );
  inFlightPasses.set(channelId, pass);
  return pass;
}

async function runPageOlderPass(
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
      // A *full* WS page that didn't advance the boundary is the dense-second
      // wall: >1 page of events share `oldestTimestamp`, so the bare `until`
      // cursor re-returns the same newest slice forever. Switch to the bridge
      // composite `(created_at, event_id)` keyset for the rest of this pass —
      // it advances within the tied second via `id > before_id` and pages all
      // older history too, so once engaged there is nothing left for WS to add.
      // An empty/short page here is transient or genuine exhaustion, not a
      // wall: fall through to break and let the scroll observer re-arm.
      if (olderMessages.length === OLDER_MESSAGES_BATCH_SIZE) {
        hasOlderMessages = await drainOlderViaKeyset({
          channelId,
          boundarySecond: oldestTimestamp,
          baseline,
          fetched,
          baselineRowCount,
          batchesFetched,
          shouldContinue,
        });
      }
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
