/**
 * Archive paging state machine for useLoadArchivedObserverEvents.
 *
 * Extracted from the hook so the two reset paths — channel change and identity
 * change — can be expressed as pure functions and exercised directly in tests
 * without a React runtime.
 *
 * The hook owns all React state/ref wrappers; this module owns the logic of
 * what gets reset under what condition.
 */

export interface ArchivePagingState {
  /** Whether the current identity has an owner_p save subscription.
   *  null = not yet checked; true/false = result of listSaveSubscriptions(). */
  hasSubscription: boolean | null;
  /** Whether older archived rows exist for the current channel. */
  hasOlderArchived: boolean;
  /** True while a fetchOlderArchived call is in flight. */
  isFetching: boolean;
  /** Backfill lifecycle: "pending" → "running" → "done". */
  backfillStatus: "pending" | "running" | "done";
  /** Promise that resolves when backfill completes. Awaited by fetchOlderArchived
   *  so the first scroll-trigger never races the index write path. */
  backfillPromise: Promise<void> | null;
  /** Resolve callback for backfillPromise. */
  backfillResolve: (() => void) | null;
  /** Compound keyset cursor: (created_at, id) of the oldest row fetched.
   *  Mirrors SQL ORDER BY created_at DESC, id DESC so same-second siblings are
   *  never skipped at a page boundary. */
  cursor: { createdAt: number; id: string } | null;
  /** True once the initial eager-hydration pass for the current channel has
   *  completed (budget reached or archive exhausted). Resets on channel change
   *  so switching channels triggers a fresh hydration pass. */
  initialHydrationDone: boolean;
  /** The channelId that this paging state is currently scoped to.
   *  Kept for diagnostics only; NOT used as a generation token (see
   *  resetGeneration). Channel equality is not a unique request identifier:
   *  A→B→A makes old-A channel checks pass again. */
  activeChannelId: string | null;
  /** Monotonically increasing counter incremented by applyChannelReset.
   *  Each fetch snapshots this value at request start and checks it again
   *  after every async boundary — a mismatch means a channel switch occurred
   *  mid-flight (even A→B→A), and results are discarded. */
  resetGeneration: number;
}

/**
 * Create a fresh ArchivePagingState with an eagerly-initialized backfill
 * promise, so fetchOlderArchived can await it before the backfill effect fires.
 */
export function createArchivePagingState(): ArchivePagingState {
  const state: ArchivePagingState = {
    hasSubscription: null,
    hasOlderArchived: true,
    isFetching: false,
    backfillStatus: "pending",
    backfillPromise: null,
    backfillResolve: null,
    cursor: null,
    initialHydrationDone: false,
    activeChannelId: null,
    resetGeneration: 0,
  };
  state.backfillPromise = new Promise<void>((resolve) => {
    state.backfillResolve = resolve;
  });
  return state;
}

/**
 * Reset per-channel paging state when the viewed channel changes.
 *
 * Only cursor, exhaustion flag, fetch lock, channel label, and generation token
 * are channel-scoped. Backfill state is identity-level (the index covers ALL
 * channels and needs to run only once per identity mount), so it is
 * intentionally NOT touched here.
 *
 * `resetGeneration` is incremented on every call. In-flight fetches snapshot
 * the generation at start and recheck it after every async boundary — a
 * mismatch (including A→B→A) means the request is stale, so results are
 * discarded. Channel ID is retained for diagnostics only; it does NOT serve
 * as the generation token.
 *
 * Called by the useEffect([channelId]) in useLoadArchivedObserverEvents.
 * Exported so tests can verify the reset semantics without a React runtime.
 */
export function applyChannelReset(
  state: ArchivePagingState,
  newChannelId: string | null,
): void {
  state.cursor = null;
  state.isFetching = false;
  state.hasOlderArchived = true;
  state.initialHydrationDone = false;
  state.activeChannelId = newChannelId;
  state.resetGeneration += 1;
}

/**
 * Run the eager initial-hydration paging loop.
 *
 * Calls `fetchOnePage()` up to `budget` times. Stops early when:
 *   - `ps.hasOlderArchived` is false (archive exhausted for this channel), OR
 *   - `signal.cancelled` is true (channel switched away mid-loop).
 *
 * `fetchOnePage` is the per-page read unit: it must respect `ps.isFetching`
 * (lock), await backfill, perform the Tauri read, ingest results, advance
 * the cursor, and set `ps.hasOlderArchived = false` when the page is short.
 * The hook wires the real implementation; tests supply a mock.
 *
 * Exported so tests can call the production loop logic directly — passing a
 * mock `fetchOnePage` — without reimplementing the control flow.
 */
export async function runHydrationLoop(
  ps: ArchivePagingState,
  fetchOnePage: () => Promise<void>,
  budget: number,
  signal: { cancelled: boolean },
): Promise<void> {
  for (let page = 0; page < budget; page++) {
    if (signal.cancelled || !ps.hasOlderArchived) {
      break;
    }
    await fetchOnePage();
  }
}
