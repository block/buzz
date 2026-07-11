/**
 * Coordination for the channel-open path.
 *
 * Two jobs, both pure module state (no React, no transport):
 *
 * 1. ORDERING — `acquireHeadFetchTicket` makes the authoritative head fetch
 *    wait for live-subscription activation when a subscription intent exists
 *    for the channel (a mounted `useChannelSubscription`). A fetch whose
 *    transport starts after the subscription is active cannot miss events in
 *    the fetch/subscribe gap, so the post-subscribe refetch that used to
 *    bridge that gap becomes unnecessary. Callers with no subscription
 *    intent (sidebar prefetch, HomeView inbox) pass immediately, unordered.
 *
 * 2. GENERATION GATING — the Tauri transport is non-abortable, so a
 *    "cancelled" unordered fetch can still resolve late. Every fetch takes a
 *    generation ticket at gate-pass; publication (window-store write) is
 *    only permitted while the ticket is still the newest. A stale unordered
 *    completion can therefore never overwrite an ordered result.
 *
 * COVERAGE — activation must decide whether a refetch is still required.
 * `lastPublication` records the (epoch, ordered) of the newest published
 * head fetch. Epochs increment per subscription intent, so an ordered fetch
 * from a previous mount session (channel A -> B -> A) fails coverage and
 * triggers exactly one background revalidation — preserving today's
 * resync-on-subscribe correctness while deleting the duplicate fetch within
 * a live session.
 *
 * Failure is open: subscription errors and gate timeouts release the fetch
 * unordered; a later activation reconciles via the coverage check.
 */

export type HeadFetchTicket = {
  generation: number;
  ordered: boolean;
  epoch: number;
};

type Waiter = (ordered: boolean) => void;

type ChannelOpenState = {
  epoch: number;
  intentPending: boolean;
  subscriptionActive: boolean;
  waiters: Set<Waiter>;
  generation: number;
  lastPublication: { epoch: number; ordered: boolean } | null;
};

const states = new Map<string, ChannelOpenState>();

function getState(channelId: string): ChannelOpenState {
  let state = states.get(channelId);
  if (!state) {
    state = {
      epoch: 0,
      intentPending: false,
      subscriptionActive: false,
      waiters: new Set(),
      generation: 0,
      lastPublication: null,
    };
    states.set(channelId, state);
  }
  return state;
}

export type SubscriptionIntent = {
  /**
   * The live subscription is active. Returns true when a head refetch is
   * required for coverage — i.e. no gated fetch is waiting to run ordered,
   * and the newest published head fetch is not ordered-in-this-epoch.
   */
  activate(): boolean;
  /** The subscription is gone (unmount or subscribe failure). Fails open. */
  dispose(): void;
};

/**
 * Registers subscription intent for a channel. MUST be called synchronously
 * when the subscription effect runs — before any await — so a head fetch
 * started by the same render waits for activation instead of racing it.
 */
export function registerSubscriptionIntent(
  channelId: string,
): SubscriptionIntent {
  const state = getState(channelId);
  state.epoch += 1;
  const epoch = state.epoch;
  state.intentPending = true;
  state.subscriptionActive = false;
  return {
    activate(): boolean {
      if (state.epoch !== epoch) return false; // superseded by a newer intent
      state.intentPending = false;
      state.subscriptionActive = true;
      const hadWaiters = state.waiters.size > 0;
      for (const waiter of [...state.waiters]) waiter(true);
      if (hadWaiters) {
        // A gated fetch resumes now; its transport starts post-activation, so
        // its eventual publication is ordered-in-this-epoch. No refetch.
        return false;
      }
      const last = state.lastPublication;
      return !(last?.ordered && last.epoch === epoch);
    },
    dispose(): void {
      if (state.epoch !== epoch) return;
      state.intentPending = false;
      state.subscriptionActive = false;
      for (const waiter of [...state.waiters]) waiter(false);
    },
  };
}

/**
 * Gate for the authoritative head fetch. Resolves with a ticket:
 * - subscription active: ordered;
 * - intent pending: when the subscription activates (ordered) or after
 *   `timeoutMs` / intent disposal (unordered — fail open);
 * - no intent (prefetch, HomeView): immediately, unordered.
 *
 * Always defers one microtask before inspecting state: on a channel open the
 * mount-triggered fetch and the subscription-intent registration run in the
 * same synchronous effect flush with the fetch first (hook order), so the
 * deferral lets the intent land and the fetch take the ordered path instead
 * of racing the subscribe.
 *
 * The timeout bounds a cold-boot hazard, not the common path: with the WS
 * still connecting, `subscribeToChannelLive` can pend for seconds, and a
 * first-ever channel open (no snapshot to paint) would hold a skeleton for
 * the whole wait. On an established socket activation lands well inside the
 * bound. A timed-out fetch publishes unordered, so activation's coverage
 * check issues the reconciling refetch — degrading exactly to today's
 * fetch-then-resync behavior.
 */
export async function acquireHeadFetchTicket(
  channelId: string,
  timeoutMs = 1_500,
): Promise<HeadFetchTicket> {
  await Promise.resolve();
  const state = getState(channelId);
  const issue = (ordered: boolean): HeadFetchTicket => {
    state.generation += 1;
    return { generation: state.generation, ordered, epoch: state.epoch };
  };
  if (state.subscriptionActive) return Promise.resolve(issue(true));
  if (!state.intentPending) return Promise.resolve(issue(false));
  return new Promise((resolve) => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const waiter: Waiter = (ordered) => {
      if (timer !== undefined) clearTimeout(timer);
      state.waiters.delete(waiter);
      resolve(issue(ordered));
    };
    state.waiters.add(waiter);
    timer = setTimeout(() => waiter(false), timeoutMs);
  });
}

/**
 * Whether a ticket is still the newest head fetch for its channel. A stale
 * ticket's fetch MUST NOT publish (window-store write) — its data was
 * superseded by a newer fetch that may already have landed.
 */
export function isCurrentHeadFetch(
  channelId: string,
  generation: number,
): boolean {
  return getState(channelId).generation === generation;
}

/** Records a completed publication for the activation coverage check. */
export function recordHeadFetchPublication(
  channelId: string,
  ticket: HeadFetchTicket,
): void {
  const state = getState(channelId);
  if (state.generation !== ticket.generation) return;
  state.lastPublication = { epoch: ticket.epoch, ordered: ticket.ordered };
}

/** Test-only: clears all per-channel state. */
export function resetChannelOpenGateForTests(): void {
  states.clear();
}
