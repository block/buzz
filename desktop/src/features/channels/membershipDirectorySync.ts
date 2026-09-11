import type { QueryClient } from "@tanstack/react-query";

import type { ChannelMember } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

const directoryQueryKey = ["relay-agents"] as const;
const COALESCE_MS = 200;
const MAX_EVENT_IDS = 256;

type PendingRefresh = {
  eventIds: Set<string>;
  timer?: ReturnType<typeof setTimeout>;
};

let refreshes = new WeakMap<QueryClient, PendingRefresh>();
const scheduled = new Set<PendingRefresh>();
let generation = 0;

/** Cancel queued work when the active community or signing identity changes. */
export function resetMembershipDirectorySync(): void {
  generation += 1;
  for (const refresh of scheduled) clearTimeout(refresh.timer);
  scheduled.clear();
  refreshes = new WeakMap();
}

/**
 * Membership changes invalidate the shared directory's channel projection.
 * Only accepted writes and membership events call this: local agent-store
 * rebuilds and policy replay must not create a directory refresh loop.
 *
 * Mark stale immediately, then coalesce bursts in a fixed window (not a sliding
 * debounce that could starve under load). Cancel even a cold in-flight read at
 * flush time: it may have started before the membership write. This refresh
 * supplies evidence, never permission or optimistic directory entries.
 */
export function refreshDirectoryAfterMembershipChange(
  queryClient: QueryClient,
  eventId?: string,
): void {
  let refresh = refreshes.get(queryClient);
  if (!refresh) {
    refresh = { eventIds: new Set() };
    refreshes.set(queryClient, refresh);
  }
  if (eventId) {
    if (refresh.eventIds.has(eventId)) return;
    refresh.eventIds.add(eventId);
    if (refresh.eventIds.size > MAX_EVENT_IDS) {
      const oldest = refresh.eventIds.values().next().value;
      if (oldest !== undefined) refresh.eventIds.delete(oldest);
    }
  }
  // Cancellation updates Query state synchronously, including cold requests.
  // Do it before marking stale so an old completion cannot look like new proof.
  void queryClient.cancelQueries({ queryKey: directoryQueryKey });
  void queryClient.invalidateQueries({
    queryKey: directoryQueryKey,
    refetchType: "none",
  });
  if (refresh.timer !== undefined) return;

  const pending = refresh;
  const scheduledGeneration = generation;
  scheduled.add(pending);
  pending.timer = setTimeout(() => {
    pending.timer = undefined;
    scheduled.delete(pending);
    const state = queryClient.getQueryState(directoryQueryKey);
    // An observer mounting during this window may already have fetched the
    // invalidated query. Do not immediately duplicate that successful read.
    if (
      state?.status === "success" &&
      !state.isInvalidated &&
      state.fetchStatus === "idle"
    )
      return;
    void queryClient.cancelQueries({ queryKey: directoryQueryKey }).then(() => {
      if (scheduledGeneration !== generation) return;
      return queryClient.invalidateQueries({ queryKey: directoryQueryKey });
    });
  }, COALESCE_MS);
}

function membershipFingerprint(members: readonly ChannelMember[]): string {
  return members
    .map(
      (member) =>
        `${normalizePubkey(member.pubkey)}:${member.role ?? ""}:${member.isAgent === true}`,
    )
    .sort()
    .join("|");
}

/**
 * A later roster can observe a write that the acceptance-time directory read
 * could not yet see. Refresh once for that semantic change, not for names,
 * ordering, repeated snapshots, or every local store notification. An initial
 * agent-bearing roster also repairs discovery when no previous roster exists.
 */
export function refreshDirectoryForRosterChange(
  queryClient: QueryClient,
  previous: readonly ChannelMember[] | undefined,
  current: readonly ChannelMember[],
): void {
  if (
    previous
      ? membershipFingerprint(previous) === membershipFingerprint(current)
      : !current.some((member) => member.isAgent || member.role === "bot")
  ) {
    return;
  }
  refreshDirectoryAfterMembershipChange(queryClient);
}
