import { useQueries } from "@tanstack/react-query";

import { getEventById } from "@/shared/api/tauri";
import { isDefinitiveEventNotFound } from "@/shared/lib/eventLookupError";

/**
 * Root-existence status for a thread-draft's parent event.
 *
 * - `checking`   — query in flight. Treated as available/counted (optimistic)
 *                  so a slow relay does not blink drafts out of the badge.
 * - `available`  — `getEventById` resolved; root exists.
 * - `deleted`    — ONLY when `get_event` returned the definitive
 *                  `"event not found"` string (relay query succeeded with zero
 *                  rows). Excluded from the badge count; open+send disabled;
 *                  "thread deleted" label shown.
 * - `error`      — any other rejection (transport/auth/serialize). Treated
 *                  as available/counted; NOT labeled deleted. Re-checked on
 *                  next panel open.
 */
export type RootStatus = "checking" | "available" | "deleted" | "error";

export function classifyError(err: unknown): RootStatus {
  return isDefinitiveEventNotFound(err) ? "deleted" : "error";
}

/**
 * Resolves root-existence status for a set of thread-root event IDs.
 *
 * **Query semantics (locked — Thufir will verify):**
 * - `staleTime: 0` — every query is always considered stale.
 * - `refetchOnMount: "always"` — re-fetch every time this hook mounts (i.e.
 *   every time the Drafts panel opens), so the flag is genuinely per-panel-open
 *   and a restored root flips `deleted` → `available` on next open.
 * - Query key = `["draftRootStatus", rootId]` — root id only, no timestamp.
 *   React Query dedupes concurrent lookups within one open; never serves an
 *   indefinitely-stale `deleted`.
 * - `deleted` is NEVER written into any persisted cache — it is a pure
 *   read-through of the current lookup.
 * - Enabled only when `isOpen` is true so we don't burn relay RTTs when the
 *   Drafts panel is closed.
 *
 * @param rootIds  Deduplicated thread-root event IDs to check.
 * @param isOpen   Whether the Drafts panel is currently visible.
 * @returns        A `Map<rootId, RootStatus>` — one entry per input root id.
 */
export function useDraftRootStatus(
  rootIds: string[],
  isOpen: boolean,
): Map<string, RootStatus> {
  const results = useQueries({
    queries: rootIds.map((rootId) => ({
      queryKey: ["draftRootStatus", rootId] as const,
      queryFn: () => getEventById(rootId),
      staleTime: 0,
      refetchOnMount: "always" as const,
      retry: false,
      enabled: isOpen && rootId.length > 0,
    })),
  });

  const statusMap = new Map<string, RootStatus>();
  rootIds.forEach((rootId, index) => {
    const result = results[index];
    if (!result) {
      statusMap.set(rootId, "checking");
      return;
    }
    if (result.isPending || result.isFetching) {
      statusMap.set(rootId, "checking");
    } else if (result.isError) {
      statusMap.set(rootId, classifyError(result.error));
    } else {
      statusMap.set(rootId, "available");
    }
  });

  return statusMap;
}
