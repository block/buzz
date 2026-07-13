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
  };
  state.backfillPromise = new Promise<void>((resolve) => {
    state.backfillResolve = resolve;
  });
  return state;
}

/**
 * Reset per-channel paging state when the viewed channel changes.
 *
 * Only cursor, exhaustion flag, and fetch lock are channel-scoped. Backfill
 * state is identity-level (the index covers ALL channels and needs to run only
 * once per identity mount), so it is intentionally NOT touched here.
 *
 * Called by the useEffect([channelId]) in useLoadArchivedObserverEvents.
 * Exported so tests can verify the reset semantics without a React runtime.
 */
export function applyChannelReset(state: ArchivePagingState): void {
  state.cursor = null;
  state.isFetching = false;
  state.hasOlderArchived = true;
}
