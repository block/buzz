import type { PresenceLookup, PresenceStatus } from "@/shared/api/types";

// Presence query keys are ["presence", ...normalizedSortedPubkeys]; a query
// "wants" an update only for a pubkey it actually requested.
export function presenceQueryWantsPubkey(
  queryKey: readonly unknown[],
  pubkey: string,
): boolean {
  return queryKey.length > 1 && queryKey.includes(pubkey);
}

// get_presence omits offline/unknown pubkeys, so a live online event often
// targets a pubkey absent from the lookup — merge it in rather than dropping it.
export function mergePresenceUpdate(
  old: PresenceLookup | undefined,
  pubkey: string,
  status: PresenceStatus,
): PresenceLookup | undefined {
  if (!old) return old;
  if (old[pubkey] === status) return old;
  return { ...old, [pubkey]: status };
}

export function getPresenceLabel(status: PresenceStatus) {
  switch (status) {
    case "online":
      return "Online";
    case "away":
      return "Away";
    case "offline":
      return "Offline";
  }
}

export function getPresenceDotClassName(status: PresenceStatus) {
  switch (status) {
    case "online":
      return "bg-emerald-500";
    case "away":
      return "bg-amber-500";
    case "offline":
      return "bg-muted-foreground/35";
  }
}
