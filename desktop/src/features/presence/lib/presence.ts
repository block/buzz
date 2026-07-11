import type { PresenceLookup, PresenceStatus } from "@/shared/api/types";

// Live kind:20001 events are self-signed by their author; the subject is
// always the event author. A p tag is NOT trusted here — a client could forge
// one to spoof another user. The relay-signed REST/seed path is the only place
// a p-tag subject is trusted. Returns null for unknown statuses.
export function parseLivePresenceEvent(event: {
  pubkey: string;
  content: string;
}): { pubkey: string; status: PresenceStatus } | null {
  const status = event.content;
  if (status !== "online" && status !== "away" && status !== "offline") {
    return null;
  }
  return { pubkey: event.pubkey.toLowerCase(), status };
}

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

// Chip styling for the presence pill (colored fill + matching text, no dot).
export function getPresenceChipClassName(status: PresenceStatus) {
  switch (status) {
    case "online":
      return "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400";
    case "away":
      return "bg-amber-500/15 text-amber-600 dark:text-amber-400";
    case "offline":
      return "bg-muted-foreground/15 text-muted-foreground";
  }
}
