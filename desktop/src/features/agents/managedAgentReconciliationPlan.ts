import type { ManagedAgentRuntimeStatus } from "@/shared/api/types";

/**
 * Pure planning core for incremental, retrying runtime reconciliation.
 *
 * `useManagedAgentRuntimeReconciliation` reconciles the configured community
 * relays against the running harness pairs. Unlike the old one-shot-per-mount
 * hook (which only re-ran on a community switch or relaunch) it must:
 *   - reconcile a newly configured relay as soon as it appears, without
 *     depending on the add flow also switching communities, and
 *   - retry a relay whose reconcile failed (relay unreachable at launch, laptop
 *     waking from sleep) with a capped backoff instead of leaving it un-spawned
 *     until the next switch or relaunch.
 *
 * These decisions live here with no React/timer/IPC dependencies so they can be
 * unit-tested directly; the hook only owns the refs, timer, and IPC calls.
 */

/** Capped retry backoff: 5s, then 30s, then 2m, then give up. */
const RETRY_BACKOFF_MS = [5_000, 30_000, 120_000] as const;

/**
 * Delay before the next retry for a relay that has failed `failureCount` times
 * (1-based: 1 = first failure). Returns null once the cap is exhausted, meaning
 * stop retrying until the community set changes again.
 */
export function reconcileRetryDelayMs(failureCount: number): number | null {
  if (failureCount < 1) return null;
  return RETRY_BACKOFF_MS[failureCount - 1] ?? null;
}

/**
 * Canonicalize the configured community relays, dropping duplicates and
 * unparsable entries. Maps canonical URL -> the raw `relayUrl` to submit to the
 * backend (first occurrence wins), so the reconcile call still speaks the
 * community's stored spelling while all bookkeeping is keyed canonically.
 */
export function canonicalCommunityRelays(
  communities: readonly { relayUrl: string }[],
  canonicalize: (url: string) => string | null,
): Map<string, string> {
  const byCanonical = new Map<string, string>();
  for (const community of communities) {
    const canonical = canonicalize(community.relayUrl);
    if (canonical === null || byCanonical.has(canonical)) continue;
    byCanonical.set(canonical, community.relayUrl);
  }
  return byCanonical;
}

/**
 * Configured relays that still need a reconcile attempt: not yet reconciled
 * cleanly and not currently in flight. Returns canonical URLs.
 */
export function pendingReconcileRelays(
  canonicalToRequested: ReadonlyMap<string, string>,
  reconciled: ReadonlySet<string>,
  inFlight: ReadonlySet<string>,
): string[] {
  const pending: string[] = [];
  for (const canonical of canonicalToRequested.keys()) {
    if (reconciled.has(canonical) || inFlight.has(canonical)) continue;
    pending.push(canonical);
  }
  return pending;
}

/**
 * Split an attempted batch into relays that reconciled cleanly vs. those that
 * still failed. A relay failed if the call threw (`rows === null` marks the
 * whole batch failed) or it produced a `failed` lifecycle row. A relay that
 * produced no rows (no eligible agents there) counts as reconciled.
 */
export function classifyReconcileResult(
  attempted: readonly string[],
  rows: readonly ManagedAgentRuntimeStatus[] | null,
  canonicalize: (url: string) => string | null,
): { succeeded: string[]; failed: string[] } {
  if (rows === null) {
    return { succeeded: [], failed: [...attempted] };
  }
  const failedRelays = new Set<string>();
  for (const row of rows) {
    if (row.lifecycle !== "failed") continue;
    const canonical = canonicalize(row.requestedRelayUrl ?? row.relayUrl);
    if (canonical !== null) failedRelays.add(canonical);
  }
  const succeeded: string[] = [];
  const failed: string[] = [];
  for (const relay of attempted) {
    if (failedRelays.has(relay)) failed.push(relay);
    else succeeded.push(relay);
  }
  return { succeeded, failed };
}
