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
 * Boot-warm gate for the *initial* forced discovery pass.
 *
 * The shared runtime catalog is in-memory only, so it starts cold every launch:
 * the cheap discovery path reports every harness `(not installed)` until a
 * forced pass warms it. Without a gate, the create/edit picker and Agents >
 * Agent defaults surfaces read that cheap path and present the cold catalog as
 * *authoritative* — blessing every harness as unavailable and blocking save —
 * during the 20–65s boot probe, and forever if that probe fails.
 *
 * This module-level state lets cheap consumers (`useAcpRuntimesQuery`) treat the
 * catalog as still-loading while the first forced pass is in flight and as a
 * retryable error if it failed, instead of authoritative. It is process-global
 * (one launch), so `startBootWarm` runs the warm exactly once no matter how many
 * times `AppShell` mounts — that also fixes the per-remount re-fire.
 *
 * The seam that protects onboarding (which renders before `AppShell` fires the
 * warm): the gate only overlays loading/error once the warm has *started*
 * (`pending`/`failed`). While `idle` — no warm yet, e.g. the onboarding flow —
 * cheap consumers behave exactly as before. A successful forced refresh from any
 * surface settles the gate, so onboarding's own forced warm clears it too.
 */
export type AcpBootWarmStatus = "idle" | "pending" | "settled" | "failed";

/**
 * A stable snapshot object for `useSyncExternalStore`: `getSnapshot` must return
 * a referentially-stable value between changes, so the object is rebuilt only in
 * `setBootWarm`, never per read.
 */
let bootWarmSnapshot: { status: AcpBootWarmStatus; error: Error | null } = {
  status: "idle",
  error: null,
};
const bootWarmListeners = new Set<() => void>();

function setBootWarm(status: AcpBootWarmStatus, error: Error | null) {
  if (bootWarmSnapshot.status === status && bootWarmSnapshot.error === error) {
    return;
  }
  bootWarmSnapshot = { status, error };
  for (const listener of bootWarmListeners) listener();
}

export function subscribeBootWarm(listener: () => void) {
  bootWarmListeners.add(listener);
  return () => {
    bootWarmListeners.delete(listener);
  };
}

export function getBootWarmSnapshot() {
  return bootWarmSnapshot;
}

/**
 * Overlay the launch boot-warm gate onto a cheap-path query result so cheap
 * consumers never present a cold catalog as authoritative. Pure so it can be
 * unit-tested without a mounted hook.
 *
 * The cheap backend response is *never* empty on a cold cache — discovery
 * always emits the full set of known runtimes (as `not_installed`/`cli_missing`
 * rows) plus presets. Gating on `data.length` would therefore be a no-op for the
 * exact payload this exists to gate, so the gate keys on the boot-warm state
 * instead and always preserves `query.data`:
 *
 * - `pending` (first forced pass in flight) reads as loading, so a cold catalog
 *   is presented as still-loading rather than a settled "everything
 *   unavailable" list — even though those cold rows are non-empty.
 * - `failed` (forced pass rejected) reads as a retryable error carrying the
 *   probe's real reason.
 * - `idle`/`settled` pass the query through unchanged, so onboarding (which
 *   renders before the warm starts) and the warmed hot path are untouched.
 *
 * `query.data` is preserved on every branch: overlaying only the lifecycle
 * flags means a consumer that reads `data ?? []` keeps its rows while a
 * status-driven consumer correctly treats them as not-yet-authoritative.
 */
export function applyBootWarmGate<
  Q extends {
    data?: unknown[];
    error: Error | null;
    isLoading: boolean;
    isPending: boolean;
    isFetching: boolean;
    isError: boolean;
  },
>(query: Q, bootWarm: { status: AcpBootWarmStatus; error: Error | null }): Q {
  if (bootWarm.status === "pending") {
    return { ...query, isLoading: true, isPending: true, isFetching: true };
  }
  if (bootWarm.status === "failed") {
    return {
      ...query,
      isError: true,
      error: bootWarm.error ?? query.error,
      isLoading: false,
    };
  }
  return query;
}

/**
 * Run the initial forced discovery pass once per launch and drive the boot-warm
 * gate. `AppShell` calls this on mount; the `pending`/`settled` short-circuit
 * makes remounts no-ops (fixing the re-fire) while still retrying after a prior
 * failure. Success is recorded by `refreshAcpRuntimes` itself (any forced
 * success settles the gate); this only has to mark its own failure.
 */
export async function startBootWarm(
  queryClient: ReturnType<typeof useQueryClient>,
) {
  const status: AcpBootWarmStatus = bootWarmSnapshot.status;
  if (status === "pending" || status === "settled") {
    return;
  }
  setBootWarm("pending", null);
  const result = await refreshAcpRuntimes(queryClient);
  // A concurrent forced success may have already settled the gate; only mark
  // failed if this pass is still the pending one and it returned no catalog.
  if (result === undefined && bootWarmSnapshot.status === "pending") {
    setBootWarm("failed", lastForcedError);
  }
}

/**
 * A stable callback that re-runs the boot warm after it failed, for the retry
 * affordance the cheap-path surfaces (create/edit picker, Agent defaults) show
 * when the gate is in its `failed` state. `startBootWarm` is the retry
 * primitive: from `failed` it transitions back through `pending` (so the
 * surface shows loading again) to `settled` on success or `failed` with a fresh
 * reason on another rejection. It no-ops while `pending`/`settled`, so a
 * double-click cannot stack probes.
 */
export function useRetryBootWarm() {
  const queryClient = useQueryClient();
  return React.useCallback(() => {
    void startBootWarm(queryClient);
  }, [queryClient]);
}

/**
 * The error from the most recent failed forced probe, surfaced through the
 * boot-warm `failed` state so a cold catalog shows a real reason rather than a
 * silent empty list. Cleared on the next forced success.
 */
let lastForcedError: Error | null = null;

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
    // Cancel and *await* the in-flight cheap query on the shared key BEFORE
    // writing the forced result. `cancelQueries` defaults to `revert: true`, so
    // cancellation restores the cheap query's pre-fetch state; doing it after
    // `setQueryData` would let that revert land last and clobber the fresh
    // forced catalog, and the gate would then settle on the stale state. With
    // the cancel awaited first, our `setQueryData` is the final write.
    await queryClient.cancelQueries({ queryKey: acpRuntimesQueryKey });
    queryClient.setQueryData(acpRuntimesQueryKey, result);
    // Any forced success proves the catalog is warm: settle the boot-warm gate
    // and clear the last error, so cheap consumers stop overlaying loading/error.
    lastForcedError = null;
    setBootWarm("settled", null);
    return result;
  } catch (error) {
    // The forced probe rejected. `fetchQuery` has already recorded the error in
    // the forced key's query state, where `useAcpRuntimesQueryForced` projects
    // it into the hook's returned `error`/`isError`. Swallow the rejection here
    // — at the single source — so the many fire-and-forget callers (mount,
    // sign-in polling, refresh buttons, and the four mutation `onSettled`
    // paths) can keep `void refreshAcpRuntimes(...)` without ever leaking an
    // unhandled rejection, and a new call site can never reintroduce one. The
    // shared cache is left untouched so consumers keep the last good catalog
    // alongside the surfaced error. Record the error so a failed boot warm can
    // surface a real reason on the cheap-path surfaces (via the boot-warm gate).
    lastForcedError = error instanceof Error ? error : new Error(String(error));
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
