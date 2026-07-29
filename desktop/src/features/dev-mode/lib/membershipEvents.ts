import { byCreatedAscending } from "@/features/dev-mode/lib/transcriptRoots";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * A member entering or leaving the channel, parsed from a relay-signed
 * kind:40099 system message. Other system message types (topic changes,
 * channel_created, …) are ignored — dev mode's transcript only narrates
 * membership.
 */
export type MembershipChange = {
  event: RelayEvent;
  change: "joined" | "added" | "left" | "removed";
  /** The member who entered or left. */
  member: string;
  /** Who added/removed them — present only when different from `member`. */
  actor: string | null;
};

export function parseMembershipEvent(
  event: RelayEvent,
): MembershipChange | null {
  if (event.kind !== KIND_SYSTEM_MESSAGE) return null;

  let payload: { type?: string; actor?: string; target?: string };
  try {
    payload = JSON.parse(event.content) as typeof payload;
  } catch {
    return null;
  }

  const actor = payload.actor ? normalizePubkey(payload.actor) : null;
  const target = payload.target ? normalizePubkey(payload.target) : null;

  switch (payload.type) {
    case "member_joined": {
      if (!target) return null;
      if (actor && actor !== target) {
        return { event, change: "added", member: target, actor };
      }
      return { event, change: "joined", member: target, actor: null };
    }
    case "member_left": {
      if (!actor) return null;
      return { event, change: "left", member: actor, actor: null };
    }
    case "member_removed": {
      if (!target) return null;
      return {
        event,
        change: "removed",
        member: target,
        actor: actor && actor !== target ? actor : null,
      };
    }
    default:
      return null;
  }
}

/** Membership changes in a channel timeline, oldest first. */
export function selectMembershipEvents(
  events: RelayEvent[] | undefined,
): MembershipChange[] {
  return (events ?? [])
    .map(parseMembershipEvent)
    .filter((change): change is MembershipChange => change !== null)
    .sort((left, right) => byCreatedAscending(left.event, right.event));
}
