import * as React from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";

import { discoverAcpRuntimes } from "@/shared/api/tauriAcpDiscovery";

/**
 * Shared React Query key for the ACP runtime catalog. Every consumer (cheap or
 * forced) reads and writes this one entry, so a forced refresh updates the same
 * cache the hot-path `useAcpRuntimesQuery` renders from.
 */
export const acpRuntimesQueryKey = ["acp-runtimes"] as const;

/**
 * Separate key for the forced (full re-discovery) fetch. Forced refresh runs on
 * *this* key, never the shared cheap key, so React Query's `fetchQuery` can
 * never deduplicate a forced probe onto an in-flight cheap request for the
 * shared key. The forced result is then written into the shared cache
 * deliberately (see `refreshAcpRuntimes`).
 */
export const acpRuntimesForcedQueryKey = ["acp-runtimes", "forced"] as const;

/**
 * Run a forced (full re-discovery) refresh and write the result into the shared
 * runtime-catalog cache.
 *
 * This is the only path that pays the expensive discovery pipeline (cache
 * clear, PATH re-fetch, CLI auth probes). Surfaces that need fresh state call
 * it deliberately: Settings/onboarding on open and on their refresh buttons,
 * and the connect/install/save/delete mutations in `onSettled`. A bare
 * `invalidateQueries` would only re-run the cheap query path and never
 * re-probe, so the freshly-changed auth/catalog state would not be reflected.
 *
 * The forced fetch runs on its own key so it can never coalesce onto an
 * in-flight *cheap* request for the shared key (which would satisfy the caller
 * with cached availability and never run the `{ force: true }` probe). Its
 * result is then written into the shared cache with `setQueryData` so hot
 * surfaces rendering `useAcpRuntimesQuery` re-render with the fresh catalog.
 * Concurrent forced callers still dedup on the forced key; the backend
 * coalesces overlapping forced runs as a second layer.
 */
export async function refreshAcpRuntimes(
  queryClient: ReturnType<typeof useQueryClient>,
) {
  try {
    const result = await queryClient.fetchQuery({
      queryKey: acpRuntimesForcedQueryKey,
      queryFn: () => discoverAcpRuntimes({ force: true }),
      staleTime: 0,
      gcTime: 0,
    });
    queryClient.setQueryData(acpRuntimesQueryKey, result);
    // A hot-surface cheap fetch may already be in flight on the shared key; cancel
    // it so its (older, cached) result cannot land after and clobber the fresh
    // forced catalog we just wrote.
    await queryClient.cancelQueries({ queryKey: acpRuntimesQueryKey });
    return result;
  } catch {
    // The forced probe rejected. `fetchQuery` has already recorded the error in
    // the forced key's query state, where `useAcpRuntimesQueryForced` projects
    // it into the hook's returned `error`/`isError`. Swallow the rejection here
    // — at the single source — so the many fire-and-forget callers (mount,
    // sign-in polling, refresh buttons, and the four mutation `onSettled`
    // paths) can keep `void refreshAcpRuntimes(...)` without ever leaking an
    // unhandled rejection, and a new call site can never reintroduce one. The
    // shared cache is left untouched so consumers keep the last good catalog
    // alongside the surfaced error.
    return undefined;
  }
}

/**
 * ACP runtimes query for surfaces that need fresh auth/version state: Settings
 * harness panels and onboarding.
 *
 * It reads the shared runtime catalog (`enabled: false`, so it never fires its
 * own cheap fetch — the forced probe below is the only fetcher) and re-renders
 * whenever `refreshAcpRuntimes` writes a fresh catalog into that cache. Loading
 * *and error* state are taken from a disabled observer on the forced key, so
 * refresh buttons and the onboarding spinner reflect the forced probe and a
 * failed probe surfaces as `error`/`isError` rather than a silent empty
 * catalog. `forceRefresh` drives explicit refresh buttons and sign-in
 * polling.
 *
 * `forceOnMount` (default `true`) is the surface owner's one force-on-mount.
 * Child rows that share the same surface must pass `forceOnMount: false`: they
 * consume the shared query state and the `forceRefresh` callback, but must not
 * mount a *second* force effect. Each mounted force effect is a distinct forced
 * probe, so an owner + N rows would otherwise re-run the 20–65s pipeline N+1
 * times on entry (and race the catalog to a later state before the owner's
 * first result renders).
 */
export function useAcpRuntimesQueryForced(options?: {
  enabled?: boolean;
  forceOnMount?: boolean;
}) {
  const enabled = options?.enabled ?? true;
  const forceOnMount = options?.forceOnMount ?? true;
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: acpRuntimesQueryKey,
    queryFn: () => discoverAcpRuntimes(),
    staleTime: 30 * 60_000,
    // Read-only observer: the forced refresh is the fetcher for these surfaces,
    // so this must never fire a cheap fetch (which would race and could
    // overwrite the fresh forced result with cached data).
    enabled: false,
  });
  // Read-only observer on the forced key so the hook surfaces the forced
  // probe's fetching *and error* state. `refreshAcpRuntimes` runs the fetch
  // imperatively via `fetchQuery`; this disabled observer never fetches itself
  // but reflects that query's state, so a rejected forced probe becomes a
  // visible `error`/`isError` instead of an unhandled rejection with a silent
  // empty/stale catalog.
  const forcedQuery = useQuery({
    queryKey: acpRuntimesForcedQueryKey,
    queryFn: () => discoverAcpRuntimes({ force: true }),
    enabled: false,
  });
  const forceRefresh = React.useCallback(
    () => refreshAcpRuntimes(queryClient),
    [queryClient],
  );
  React.useEffect(() => {
    if (enabled && forceOnMount) void forceRefresh();
  }, [enabled, forceOnMount, forceRefresh]);
  const isFetching = query.isFetching || forcedQuery.isFetching;
  return {
    ...query,
    error: forcedQuery.error ?? query.error,
    isError: forcedQuery.isError || query.isError,
    isFetching,
    isLoading: isFetching && query.data === undefined,
    forceRefresh,
  };
}
