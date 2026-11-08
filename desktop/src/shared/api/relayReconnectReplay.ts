import { CHANNEL_EVENT_KINDS } from "@/shared/constants/kinds";
import type {
  RelaySubscription,
  RelaySubscriptionFilter,
} from "@/shared/api/relayClientShared";
import type { RelayEvent } from "@/shared/api/types";
import {
  markReconnectRepairDone,
  shouldDispatchSubscriptionEvent,
} from "@/shared/api/relayClosedRecovery";
import type { ChannelReconnectRepairRequest } from "@/shared/api/channelReconnectRepair";
import {
  isRateLimited,
  waitForRateLimit,
} from "@/shared/api/relayRateLimitGate";
import {
  computeResumeSince,
  RELAY_INGEST_FUTURE_TOLERANCE_SECS,
  stampLiveUntilOnClose,
} from "@/shared/api/relayResumeCursor";

export { RELAY_INGEST_FUTURE_TOLERANCE_SECS };

/**
 * Legacy cursors are event-time maxima, so cover relay future tolerance (900s)
 * plus the channel-row storage fence and margin (960s + 5s).
 */
export const DB_CREATED_AT_FLOOR_SECS = 960;
export const FENCE_CLOCK_MARGIN_SECS = 5;
export const RECONNECT_REPLAY_CHANNEL_LOOKBACK_SECS =
  RELAY_INGEST_FUTURE_TOLERANCE_SECS +
  DB_CREATED_AT_FLOOR_SECS +
  FENCE_CLOCK_MARGIN_SECS;
export const RECONNECT_REPLAY_PAGE_LIMIT = 500;
export const RECONNECT_REPLAY_PAGE_CONCURRENCY = 4;

/**
 * Maximum attempts for one subscription's paged history backfill.
 *
 * Backfill failures must never escape `replayLiveSubscriptions`: by the time
 * paging starts, every live REQ has already been re-established on a healthy,
 * authenticated socket. Letting a history rejection propagate makes the
 * session tear that socket down (`resetConnection`) and reconnect straight
 * into the same rate-limit window — the "briefly connected → can't reach the
 * relay" flap loop. Instead each sub retries behind the rate-limit gate a
 * bounded number of times, then degrades to live-only for this connection.
 * The window's lower bound is pinned in `pendingReplaySince` while unresolved
 * (live events advance `lastSeenCreatedAt` regardless of backfill success),
 * so the next reconnect still requests the missed window.
 */
export const PAGE_REPLAY_MAX_ATTEMPTS = 3;

/**
 * Maximum live subscriptions sent per relay REQ burst during reconnect.
 *
 * Capping the initial blast prevents admission-control bursts on degraded
 * networks where the relay is already near its per-pubkey quota.
 */
export const REPLAY_BATCH_SIZE = 8;

/**
 * Delay between consecutive replay batches (milliseconds).
 *
 * Spreads the REQ storm across time so the relay's sliding quota window
 * can absorb each batch without triggering rate-limiting on the next.
 */
export const REPLAY_INTER_BATCH_DELAY_MS = 50;

/**
 * Longest a paged channel's HTTP repair waits for its live-only REQ to be
 * acknowledged (EOSE) before treating the attempt as failed.
 *
 * The repair must start after the relay registered the live subscription:
 * the live REQ no longer reads history, so an event stored between the
 * repair's upper bound and registration would be delivered by neither.
 */
export const LIVE_REQ_ACK_TIMEOUT_MS = 25_000;

async function runWithConcurrency<T>(
  items: T[],
  concurrency: number,
  worker: (item: T) => Promise<void>,
) {
  const workerCount = Math.min(Math.max(1, concurrency), items.length);
  let nextIndex = 0;

  await Promise.all(
    Array.from({ length: workerCount }, async () => {
      while (nextIndex < items.length) {
        const item = items[nextIndex++];
        await worker(item);
      }
    }),
  );
}

/**
 * Resolves `true` once the restored live REQ reached EOSE on this replay's
 * connection, `false` on timeout. Also resolves early (`true`) when this
 * replay is superseded or its connection or subscription is retired; callers
 * re-check `isActive()` and subscription identity after waking. A waiter that
 * outlived its connection would hold `connect()` (and with it the session's
 * `connectPromise`) for the whole timeout, stalling the scheduled reconnect.
 */
function awaitLiveEose(
  replay: NonNullable<
    Extract<RelaySubscription, { mode: "live" }>["reconnectReplay"]
  >,
  isActive: () => boolean,
  setTimeoutFn: (fn: () => void, ms: number) => number,
  clearTimeoutFn: (handle: number) => void,
): Promise<boolean> {
  if (replay.liveEose || !isActive()) return Promise.resolve(true);
  // The deadline runs from the REQ's send, not from when a (concurrency
  // limited) worker got here: a queued channel must not get a fresh wait.
  const sentAtMs = replay.liveReqSentAtMs ?? Date.now();
  const remainingMs = Math.max(
    0,
    sentAtMs + LIVE_REQ_ACK_TIMEOUT_MS - Date.now(),
  );
  return new Promise<boolean>((resolve) => {
    const timer = setTimeoutFn(() => {
      replay.liveEoseWaiter = undefined;
      resolve(false);
    }, remainingMs);
    replay.liveEoseWaiter = () => {
      replay.liveEoseWaiter = undefined;
      clearTimeoutFn(timer);
      resolve(true);
    };
  });
}

/** Wake a repair worker parked on this subscription's live EOSE, if any. */
export function releaseLiveEoseWaiter(subscription: RelaySubscription) {
  if (subscription.mode !== "live") return;
  subscription.reconnectReplay?.liveEoseWaiter?.();
}

/**
 * Per-subscription bookkeeping for a connection that is being torn down:
 * stamp the resume cursor, then release every repair worker parked on that
 * connection's EOSE so the stale `connect()` settles at once.
 */
export function retireConnectionSubscriptions(
  subscriptions: Map<string, RelaySubscription>,
  generation: number,
  lastInboundMs: number,
) {
  stampLiveUntilOnClose(subscriptions, generation, lastInboundMs);
  for (const subscription of subscriptions.values()) {
    releaseLiveEoseWaiter(subscription);
  }
}

export function buildReconnectReplayFilter(
  filter: RelaySubscriptionFilter,
  since?: number,
  until?: number,
  limit = Math.min(filter.limit, RECONNECT_REPLAY_PAGE_LIMIT),
) {
  if (since === undefined) return filter;

  const replayFilter: RelaySubscriptionFilter = {
    ...filter,
    limit,
    since: filter.since === undefined ? since : Math.max(filter.since, since),
  };

  if (until !== undefined) {
    replayFilter.until =
      filter.until === undefined ? until : Math.min(filter.until, until);
  }

  return replayFilter;
}

export function shouldPageReconnectReplay(filter: RelaySubscriptionFilter) {
  return (
    filter.limit > 0 &&
    Array.isArray(filter["#h"]) &&
    filter["#h"].length === 1 &&
    CHANNEL_EVENT_KINDS.every((kind) => filter.kinds.includes(kind))
  );
}

/**
 * Page one subscription's missed-window history.
 *
 * Returns `true` only when the window was genuinely completed (short page or
 * boundary reached). Returns `false` when the pass aborted because the
 * connection went stale (`isActive()` false) — callers must NOT treat that as
 * completion: the same subscription object is shared with the superseding
 * connection, and clearing its pinned `pendingReplaySince` on a stale abort
 * would erase the floor the new connection still needs.
 */
export async function replayReconnectHistoryPages({
  subscription,
  channelId,
  since,
  until,
  isActive,
  requestRepair,
}: {
  subscription: Extract<RelaySubscription, { mode: "live" }>;
  channelId: string;
  since: number;
  until?: number;
  isActive: () => boolean;
  requestRepair: (
    request: ChannelReconnectRepairRequest,
  ) => Promise<RelayEvent[]>;
}): Promise<boolean> {
  let pageUntil: number | undefined = until;
  let beforeId: string | undefined;

  while (pageUntil === undefined || pageUntil >= since) {
    if (!isActive()) return false;

    const events = await requestRepair({
      channelId,
      since,
      limit: RECONNECT_REPLAY_PAGE_LIMIT,
      until: pageUntil,
      beforeId,
    });

    if (!isActive()) return false;

    for (const event of events) {
      if (shouldDispatchSubscriptionEvent(subscription, event)) {
        subscription.onEvent(event);
      }
    }
    if (events.length < RECONNECT_REPLAY_PAGE_LIMIT) return true;

    const terminal = events.at(-1);
    if (!terminal || terminal.created_at < since) return true;
    if (terminal.created_at === pageUntil && terminal.id === beforeId) {
      throw new Error("Reconnect repair cursor did not advance");
    }
    if (terminal.created_at === since) {
      // Continue across an arbitrarily dense boundary second until the relay
      // returns a short page; the composite cursor, not timestamp subtraction,
      // guarantees progress.
      pageUntil = since;
    } else {
      pageUntil = terminal.created_at;
    }
    beforeId = terminal.id;
  }
  return true;
}

export async function replayLiveSubscriptions({
  subscriptions,
  sendRaw,
  requestRepair,
  generation = 0,
  pageReplayConcurrency = RECONNECT_REPLAY_PAGE_CONCURRENCY,
  visibleChannelId = null,
  replayBatchSize = REPLAY_BATCH_SIZE,
  interBatchDelayMs = REPLAY_INTER_BATCH_DELAY_MS,
  setTimeoutFn = (fn: () => void, ms: number) =>
    window.setTimeout(fn, ms) as unknown as number,
  clearTimeoutFn = (handle: number) => window.clearTimeout(handle),
  isActive = () => true,
}: {
  subscriptions: Map<string, RelaySubscription>;
  sendRaw: (payload: unknown[]) => Promise<void>;
  requestRepair: (
    request: ChannelReconnectRepairRequest,
  ) => Promise<RelayEvent[]>;
  generation?: number;
  pageReplayConcurrency?: number;
  /** Channel currently visible in the UI — its subscriptions go in the first batch. */
  visibleChannelId?: string | null;
  /** Max subscriptions per REQ burst (injectable for tests). */
  replayBatchSize?: number;
  /** Milliseconds between bursts (injectable for tests). */
  interBatchDelayMs?: number;
  /** setTimeout implementation (injectable for tests). */
  setTimeoutFn?: (fn: () => void, ms: number) => number;
  /** clearTimeout matching `setTimeoutFn` (injectable for tests). */
  clearTimeoutFn?: (handle: number) => void;
  /**
   * Returns false when the connection that initiated this replay has been
   * superseded by a newer one. After the gate await resumes, a stale replay
   * must not double-send REQs on the live socket.
   */
  isActive?: () => boolean;
}) {
  // If the relay has signalled back-pressure, wait for the gate to clear
  // before blasting a full set of REQs that would immediately be rate-limited.
  if (isRateLimited()) await waitForRateLimit();

  // A newer connection may have replayed while this one was suspended at the
  // gate — abort silently to avoid double-sending every REQ on the live socket.
  if (!isActive()) return;

  const replayRequests = Array.from(subscriptions.entries())
    .filter(
      (
        entry,
      ): entry is [string, Extract<RelaySubscription, { mode: "live" }>] =>
        entry[1].mode === "live",
    )
    .map(([subId, subscription]) => {
      const channelId = subscription.filter["#h"]?.[0];
      const shouldPageReplay =
        channelId !== undefined &&
        shouldPageReconnectReplay(subscription.filter);
      // Paged channels keep the wide event-time lookback for both terms: the
      // relay accepts `created_at` up to 900s either side of storage time, so
      // a narrower window could skip a backdated row stored after the close.
      const cursorSince = computeResumeSince({
        lastSeenCreatedAt: subscription.lastSeenCreatedAt,
        liveUntil: subscription.liveUntil,
        ...(shouldPageReplay && {
          lastSeenSkewSecs: RECONNECT_REPLAY_CHANNEL_LOOKBACK_SECS,
          liveUntilSkewSecs: RECONNECT_REPLAY_CHANNEL_LOOKBACK_SECS,
        }),
      });
      // A pinned floor from a previously failed backfill takes precedence
      // over the cursor: live events kept advancing `lastSeenCreatedAt`
      // while the older window stayed unresolved, and starting from the
      // cursor would skip it permanently.
      const replayFloor =
        cursorSince === undefined
          ? subscription.pendingReplaySince
          : Math.min(cursorSince, subscription.pendingReplaySince ?? Infinity);
      // Native repair bypasses the original WS filter, so preserve its lower
      // bound explicitly. Otherwise a from-now subscription can replay older
      // rows and surface stale unread or notification activity on reconnect.
      const replaySince =
        replayFloor === undefined
          ? undefined
          : Math.max(replayFloor, subscription.filter.since ?? 0);
      const willRepair = shouldPageReplay && replaySince !== undefined;
      let replay: typeof subscription.reconnectReplay;
      if (willRepair) {
        // Pin the window's lower bound NOW, before any live-only REQ is sent
        // or any await: every live REQ goes out before the (concurrency
        // limited) repair workers start, so a queued channel can reach EOSE
        // and have its socket close stamp `liveUntil` while its history was
        // never fetched. The pin keeps the next replay's floor at or below
        // this one until a repair pass actually completes. Events on the
        // live REQ advance `lastSeenCreatedAt` independently of backfill
        // success, so the cursor alone could skip the window permanently.
        subscription.pendingReplaySince = replaySince;
        // A superseded replay's worker is parked on its own state object.
        subscription.reconnectReplay?.liveEoseWaiter?.();
        // Install before the restored live REQ: a live frame may beat page one.
        replay = subscription.reconnectReplay = {
          generation,
          seenEventIds: new Set(),
          liveEose: false,
          repairDone: false,
        };
      }

      return {
        subId,
        subscription,
        channelId,
        replaySince,
        replay,
        shouldPageReplay: willRepair,
        // Quiet subscriptions resume from `liveUntil` alone; keep their full
        // original limit rather than the steady-state cap.
        resumeLimit:
          subscription.lastSeenCreatedAt === undefined
            ? subscription.filter.limit
            : undefined,
      };
    });

  // Visible timeline and interactive consumers (including off-screen huddle
  // speech) recover before cold work. Stable sort preserves registration order
  // within each tier, including ties between visible and interactive owners.
  const isForeground = (
    subscription: Extract<RelaySubscription, { mode: "live" }>,
  ) =>
    subscription.priority === "interactive" ||
    (visibleChannelId !== null &&
      (subscription.filter["#h"]?.includes(visibleChannelId) ?? false));
  replayRequests.sort(
    (a, b) =>
      Number(isForeground(b.subscription)) -
      Number(isForeground(a.subscription)),
  );

  // Send live REQs in capped batches with inter-batch delays to avoid
  // triggering per-pubkey admission control on degraded/recovering connections.
  for (let i = 0; i < replayRequests.length; i += replayBatchSize) {
    // Re-check the gate before every batch: a previous batch may have triggered
    // admission control and armed the gate mid-replay. Wait for it to clear,
    // then verify the connection is still current — a newer connection may have
    // replayed while we were suspended.
    if (isRateLimited()) await waitForRateLimit();
    if (!isActive()) return;
    const batch = replayRequests.slice(i, i + replayBatchSize);
    await Promise.all(
      batch.map(
        async ({
          subId,
          subscription,
          replaySince,
          shouldPageReplay,
          resumeLimit,
          replay,
        }) => {
          // The subscription map may change while replay is paused behind the
          // gate or an inter-batch delay. Never resurrect a disposed entry (or
          // overwrite its replacement) with a REQ from this stale snapshot.
          if (!isActive() || subscriptions.get(subId) !== subscription) return;
          if (replay) replay.liveReqSentAtMs = Date.now();
          await sendRaw([
            "REQ",
            subId,
            // The HTTP repair below already covers the missed window, so the
            // live REQ reads no history (`limit: 0`); the repair starts only
            // after this REQ's EOSE, which keeps backfill and live overlapping.
            shouldPageReplay
              ? { ...subscription.filter, limit: 0 }
              : buildReconnectReplayFilter(
                  subscription.filter,
                  replaySince,
                  undefined,
                  resumeLimit,
                ),
          ]);
          // Disposal can race the asynchronous websocket invoke after the
          // identity check. If this exact entry was deleted while REQ was in
          // flight, CLOSE again after REQ settles so wire order converges to
          // closed. A different object under the same ID is a replacement and
          // must not be torn down by this stale replay.
          if (isActive() && !subscriptions.has(subId)) {
            await sendRaw(["CLOSE", subId]);
          }
        },
      ),
    );
    if (i + replayBatchSize < replayRequests.length) {
      await new Promise<void>((resolve) =>
        setTimeoutFn(resolve, interBatchDelayMs),
      );
    }
  }

  await runWithConcurrency(
    replayRequests.filter(
      (
        request,
      ): request is typeof request & {
        channelId: string;
        replaySince: number;
        replay: NonNullable<typeof request.replay>;
        shouldPageReplay: true;
      } =>
        request.shouldPageReplay &&
        request.channelId !== undefined &&
        request.replaySince !== undefined,
    ),
    pageReplayConcurrency,
    async ({ subId, subscription, channelId, replaySince, replay }) => {
      // Backfill is best-effort: a failure here (typically a `rate-limited:`
      // CLOSED on a history REQ) must never escape to the session and tear
      // down the healthy, authenticated socket carrying the live REQs — that
      // is the connect→drop flap loop. Retry behind the gate a bounded number
      // of times, then fall back to a bounded history REQ.
      //
      // The window's lower bound was pinned when the replay state was built;
      // it is cleared only on a completed pass below.
      const isCurrent = () =>
        isActive() && subscriptions.get(subId) === subscription;

      // The live REQ reads no history (`limit: 0`) and the pinned floor only
      // helps the next reconnect, so without this the missed window stays
      // empty for the rest of a stable connection. Fall back to the
      // pre-repair behavior: re-issue the REQ (same ID, so the relay replaces
      // the live subscription) with the window's bounded history. The floor
      // stays pinned for anything the cap left out.
      const sendHistoryFallback = async () => {
        // That history overlaps what live (and any completed page) already
        // dispatched, so keep the seen IDs until the fallback's own EOSE. If
        // the original EOSE was consumed, the next one for this generation is
        // the fallback's; if it never came, a late one could pass for the
        // fallback's, so only the deadline below ends dedupe.
        replay.repairDone = true;
        if (replay.liveEose) replay.liveEose = false;
        else replay.ignoreEose = true;
        if (isRateLimited()) await waitForRateLimit();
        // Dedupe ends on that EOSE, on teardown (both call the waiter), or at
        // the deadline: the relay may drop an EOSE without closing the
        // socket, and the set must not grow with live traffic for the rest of
        // the connection. Identity-fenced so a stale release cannot clear a
        // successor's state.
        let deadline: number | undefined;
        const endDedupe = () => {
          if (deadline !== undefined) clearTimeoutFn(deadline);
          replay.liveEoseWaiter = undefined;
          if (subscription.reconnectReplay === replay) {
            subscription.reconnectReplay = undefined;
          }
        };
        if (!isCurrent()) {
          endDedupe();
          return;
        }
        replay.liveEoseWaiter = endDedupe;
        deadline = setTimeoutFn(endDedupe, LIVE_REQ_ACK_TIMEOUT_MS);
        try {
          await sendRaw([
            "REQ",
            subId,
            buildReconnectReplayFilter(subscription.filter, replaySince),
          ]);
        } catch (sendError: unknown) {
          endDedupe();
          console.warn(
            `[reconnect replay] history fallback REQ failed for ${subId}:`,
            sendError,
          );
          return;
        }
        // Same convergence as the restored live REQ: a disposal that raced
        // this send must still end closed on the wire.
        if (isActive() && !subscriptions.has(subId)) {
          await sendRaw(["CLOSE", subId]).catch(() => {});
        }
      };

      // Take the repair's upper bound only after the relay acknowledged the
      // live-only REQ (registered it), never before. One deadline per REQ:
      // waiting again on the same unchanged REQ cannot make it arrive.
      const acked = await awaitLiveEose(
        replay,
        isCurrent,
        setTimeoutFn,
        clearTimeoutFn,
      );
      if (!isCurrent()) return;
      if (!acked) {
        console.warn(
          `[reconnect replay] live REQ for ${subId} was not acknowledged; falling back to bounded history`,
        );
        await sendHistoryFallback();
        return;
      }
      for (let attempt = 1; attempt <= PAGE_REPLAY_MAX_ATTEMPTS; attempt++) {
        try {
          const completed = await replayReconnectHistoryPages({
            subscription,
            channelId,
            since: replaySince,
            // Do not trust the renderer clock as the upper bound. The relay's
            // newest matching row starts the keyset walk.
            until: undefined,
            // Both guards are required. The identity check catches the sub
            // being torn down/replaced; the outer isActive() catches
            // connection supersession, which bumps the generation while the
            // SAME subscription key and object survive in the map — identity
            // alone stays true and a stale pass could complete and clear the
            // floor the superseding connection needs.
            isActive: isCurrent,
            requestRepair,
          });
          // A stale-connection abort is NOT completion: the superseding
          // connection shares this subscription object and still needs the
          // pinned floor for its own replay. Only a genuinely completed
          // window may release it.
          if (completed) {
            subscription.pendingReplaySince = undefined;
            markReconnectRepairDone(subscription, generation);
          }
          return;
        } catch (error) {
          console.warn(
            `[reconnect replay] history backfill attempt ${attempt}/${PAGE_REPLAY_MAX_ATTEMPTS} failed for ${subId}:`,
            error,
          );
          if (attempt === PAGE_REPLAY_MAX_ATTEMPTS) {
            await sendHistoryFallback();
            return;
          }
          // The failed REQ's CLOSED handler arms the rate-limit gate before
          // rejecting; wait for it (no-op when the failure wasn't back-pressure)
          // and re-check that this replay's connection is still current.
          if (isRateLimited()) await waitForRateLimit();
          if (!isCurrent()) return;
        }
      }
    },
  );
}
