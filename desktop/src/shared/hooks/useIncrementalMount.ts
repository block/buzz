import * as React from "react";

/**
 * Mounts a long list progressively: the first `initial` items render in the
 * first commit (fast click→paint), then the count grows by `step` in chained
 * low-priority transitions until everything is mounted. This bounds the cost
 * of any single React commit — a one-shot mount of hundreds of heavy rows
 * blocked the main thread for 0.5–1.5s on the Projects tabs.
 *
 * The count persists across in-place data updates (a refetch re-rendering an
 * already-grown list stays fully mounted) and resets naturally when the
 * consuming component remounts (tab switches swap subtrees).
 *
 * Two guards keep the single-commit bound honest without a remount:
 * - `enabled: false` (the inactive layout of a list/grid pair) holds the
 *   count at `initial`, so switching layouts mounts the other collection
 *   incrementally instead of all at once from its already-grown counter.
 * - A `total` shrink (scope narrowed) snaps the count down — everything
 *   visible stays mounted — so restoring the wider scope re-grows stepwise
 *   instead of bulk-mounting the whole collection in one commit.
 */
export function useIncrementalMount(
  total: number,
  initial = 30,
  step = 60,
  enabled = true,
): number {
  const [count, setCount] = React.useState(initial);

  React.useEffect(() => {
    if (!enabled) {
      if (count !== initial) setCount(initial);
      return;
    }
    if (total < count) {
      setCount(Math.max(initial, total));
      return;
    }
    if (count >= total) return;
    let cancelled = false;
    // rAF so a frame paints between growth steps; the transition keeps each
    // growth commit interruptible by user input.
    const frame = window.requestAnimationFrame(() => {
      if (cancelled) return;
      React.startTransition(() => {
        setCount((current) => Math.min(current + step, total));
      });
    });
    return () => {
      cancelled = true;
      window.cancelAnimationFrame(frame);
    };
  }, [count, enabled, initial, step, total]);

  return Math.min(count, total);
}

/**
 * Trims grouped rows to a total row budget, preserving group order and
 * dropping empty groups. Pure core for incrementally-mounted grouped lists.
 */
export function sliceGroupedRows<G extends { rows: readonly unknown[] }>(
  groups: readonly G[],
  limit: number,
): G[] {
  const result: G[] = [];
  let remaining = limit;
  for (const group of groups) {
    if (remaining <= 0) break;
    if (group.rows.length <= remaining) {
      result.push(group);
      remaining -= group.rows.length;
    } else {
      result.push({ ...group, rows: group.rows.slice(0, remaining) });
      remaining = 0;
    }
  }
  return result;
}

/** Total row count across groups. */
export function countGroupedRows(
  groups: readonly { rows: readonly unknown[] }[],
): number {
  return groups.reduce((sum, group) => sum + group.rows.length, 0);
}
