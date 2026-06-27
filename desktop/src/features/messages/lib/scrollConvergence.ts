/**
 * Pure staleness + termination decision for scrolling a virtualized timeline to
 * a message that may be far off-screen.
 *
 * @tanstack/react-virtual already owns the OFFSET convergence: a single
 * `scrollToIndex(index)` captures that index in `scrollState`, and its internal
 * `reconcileScroll` rAF loop re-runs `getOffsetForIndex(index)` every frame —
 * re-aiming as off-screen rows mount and `measureElement` corrects their
 * heights — until the offset is stable (or a 5s safety valve fires). We do NOT
 * recompute offsets; duplicating `getOffsetForIndex` against the library's own
 * `measurementsCache`/`scrollMargin`/`scrollPadding` would only drift.
 *
 * What the library does NOT do: it chases the INDEX captured at call time, with
 * no concept of a message id. If the data shifts mid-settle — a prepend or a
 * delete above the target — the captured index now points at the wrong row and
 * the library happily settles on it. This reducer owns exactly that gap: each
 * frame it re-resolves the target's CURRENT index from the live map and decides
 * whether the adapter must re-aim the library, let it settle, or stop.
 *
 * Two correctness properties this enforces and the `.mjs` suite gates:
 *  - The target index is re-resolved by id every frame (never frozen), so a
 *    concurrent prepend/delete that shifts the target re-aims the library at the
 *    new index instead of stranding it on the old one.
 *  - If the target id leaves the data mid-settle (deleted), the loop terminates
 *    with `converged: false` rather than chasing a vanished row to the cap.
 *
 * Plus one liveness property: a large windowed-out jump can leave the library's
 * own reconcile deadlocked off-target (offset stable but short of the target
 * after rows re-measured under it). When that happens the reducer signals a
 * same-index re-issue (`reissue: true`) to restart the library's reconcile —
 * the single case where re-issuing an unchanged index is correct.
 */

/** Where a scroll target should land in the viewport. Mirrors the library's align. */
export type ConvergenceAlign = "start" | "center" | "end";

export type ConvergenceInput = {
  /** Id of the message to settle on — re-resolved against the map each frame. */
  targetMessageId: string;
  /** Live message-id -> item-index map; re-read every frame (staleness guard). */
  indexByMessageId: Map<string, number>;
  /**
   * Index the library is currently chasing (the last index the adapter issued
   * via `scrollToIndex`), or `null` before the first issue. Lets the reducer
   * tell a re-aim (index moved) from a steady settle (index unchanged).
   */
  lastIssuedIndex: number | null;
  /**
   * Whether the library's offset reached the current index's target this frame
   * and stopped moving. Only meaningful once the library is chasing the CURRENT
   * index; a settle reported while re-aiming is ignored.
   */
  librarySettled: boolean;
  /**
   * Whether the library's offset has stopped moving but is NOT at the current
   * index's target — it stalled mid-reconcile (its internal re-aim deadlocked
   * after rows re-measured under it). The reducer kicks it with a fresh re-issue
   * at the same index. Mutually exclusive with `librarySettled`.
   */
  stalledOffTarget: boolean;
  /** Frames already spent in the loop (the adapter increments per rAF). */
  framesUsed: number;
};

export type ConvergenceDecision = {
  /**
   * Index the adapter should be aiming the library at, or `null` when the
   * target is gone. The adapter only re-issues `scrollToIndex` when this differs
   * from `lastIssuedIndex`, so a steady settle issues no redundant scroll (which
   * would reset the library's `stableFrames` and prevent it from ever settling).
   */
  nextIndex: number | null;
  /**
   * True when the adapter must re-issue `scrollToIndex(nextIndex)` even though
   * the index is unchanged — used to kick the library out of an off-target
   * stall. A normal steady settle leaves this false so no redundant scroll
   * resets the library's `stableFrames`.
   */
  reissue: boolean;
  /** True once the loop must stop (settled, target gone, or frame cap hit). */
  done: boolean;
  /** True only when the loop stopped because the target row actually settled. */
  converged: boolean;
};

/**
 * Hard cap on frames so a perpetually re-measuring row, or a target whose index
 * keeps shifting, can't spin the loop forever. The library has its own 5s valve;
 * this is the adapter-side bound expressed in frames for deterministic testing.
 */
export const CONVERGENCE_FRAME_CAP = 32;

/**
 * One frame of the convergence loop. Pure: given the live map and the library's
 * settle state, decides the index to aim at and whether to stop.
 */
export function convergenceStep(input: ConvergenceInput): ConvergenceDecision {
  const currentIndex = input.indexByMessageId.get(input.targetMessageId);

  // Target left the data mid-settle (deleted) — stop without converging so the
  // adapter clears the highlight instead of chasing a vanished row.
  if (currentIndex === undefined) {
    return { nextIndex: null, reissue: false, done: true, converged: false };
  }

  const aimingAtCurrent = input.lastIssuedIndex === currentIndex;

  // The library only settles meaningfully once it is chasing the CURRENT index.
  // A settle reported while we are still re-aiming (index just moved) is stale.
  if (aimingAtCurrent && input.librarySettled) {
    return {
      nextIndex: currentIndex,
      reissue: false,
      done: true,
      converged: true,
    };
  }

  // Frame cap: accept the best index we have rather than spin forever on a row
  // whose height never settles or a target whose index keeps shifting.
  if (input.framesUsed + 1 >= CONVERGENCE_FRAME_CAP) {
    return {
      nextIndex: currentIndex,
      reissue: false,
      done: true,
      converged: false,
    };
  }

  // Library stalled off-target: its offset stopped moving but never reached the
  // current index (its internal reconcile deadlocked after rows re-measured).
  // Re-issue the SAME index to restart its reconcile — the one case where a
  // same-index re-issue is correct rather than a stableFrames-resetting bug.
  if (aimingAtCurrent && input.stalledOffTarget) {
    return {
      nextIndex: currentIndex,
      reissue: true,
      done: false,
      converged: false,
    };
  }

  // Either the index moved (adapter will re-issue scrollToIndex) or the library
  // is still settling on the current index (adapter issues nothing, just waits).
  return {
    nextIndex: currentIndex,
    reissue: false,
    done: false,
    converged: false,
  };
}
