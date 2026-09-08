/**
 * Grouped-membership payload construction for system rows.
 *
 * `buildTimelineItems` decides *which* contiguous membership events collapse
 * into one `system-group`; this module decides *how* that group is described.
 * The two rules are one invariant, so they live where both the renderer and a
 * lib-level test can import them — a group the builder emits but this module
 * cannot describe silently degrades to "render only the oldest event", which
 * drops the rest of the group from the timeline.
 *
 * Kept pure (no React, no DOM) so `membershipGroupPayload.test.mjs` and
 * `timelineItems.test.mjs` can cross-check the two rules directly.
 */

import type { TimelineMessage } from "@/features/messages/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

/** Parsed body of a kind:40099 system message, plus the synthetic group kinds. */
export type SystemMessagePayload = {
  type: string;
  actor?: string;
  arrivals?: Array<{ actor: string; target: string }>;
  target?: string;
  targets?: string[];
  topic?: string;
  purpose?: string;
  // Moderation tombstone fields (kind:40099 "message_deleted"). All optional and
  // moderator-authored — present when a moderator removed the message, absent for
  // a plain member self-delete. Reporter identity/evidence never appears here.
  public_reason?: string;
  reason_code?: string;
  action_id?: string;
};

/** Parses one system message body; returns null when the body is not JSON. */
export function parseSystemMessagePayload(
  message: TimelineMessage,
): SystemMessagePayload | null {
  try {
    return JSON.parse(message.body) as SystemMessagePayload;
  } catch {
    return null;
  }
}

/**
 * Derives the synthetic payload describing a grouped membership row, or null
 * when the group is not a shape this module can describe.
 *
 * Must stay total over everything `buildTimelineItems` groups —
 * `membershipGroupPayload.test.mjs` asserts that with a matrix invariant.
 */
export function buildGroupedMembershipPayload(
  messages: readonly TimelineMessage[],
): SystemMessagePayload | null {
  if (messages.length < 2) return null;

  const payloads = messages.map(parseSystemMessagePayload);
  const joinedThenLeft = buildJoinedThenLeftPayload(payloads);
  if (joinedThenLeft) return joinedThenLeft;

  const arrivals = payloads.map((payload) => {
    const payloadActor = payload?.actor ? normalizePubkey(payload.actor) : null;
    const payloadTarget = payload?.target
      ? normalizePubkey(payload.target)
      : null;
    if (payload?.type !== "member_joined" || !payloadActor || !payloadTarget) {
      return null;
    }
    return { actor: payloadActor, target: payloadTarget };
  });
  if (arrivals.some((arrival) => !arrival)) return null;

  const membershipArrivals = arrivals as Array<{
    actor: string;
    target: string;
  }>;
  const targets = [...new Set(membershipArrivals.map(({ target }) => target))];
  return {
    arrivals: membershipArrivals,
    type: "members_arrived",
    target: targets[0],
    targets,
  };
}

/**
 * One lifecycle summary for N>=1 equivalent self-arrivals of a member followed
 * by that same member departing.
 *
 * Mirrors `membershipChangesCanGroup`: a departure only groups with self-arrivals
 * of the departing member, and the builder absorbs *every* contiguous one, not
 * just the nearest. Duplicate `member_joined` events are a real relay artifact —
 * `handle_put_user` re-emits one on each PUT_USER — so pinning this to a 2-event
 * pair dropped the departure from the timeline entirely.
 */
function buildJoinedThenLeftPayload(
  payloads: readonly (SystemMessagePayload | null)[],
): SystemMessagePayload | null {
  if (payloads.length < 2) return null;

  const departure = payloads[payloads.length - 1];
  const departureActor = departure?.actor
    ? normalizePubkey(departure.actor)
    : null;
  if (departure?.type !== "member_left" || !departureActor) return null;

  const arrivals = payloads.slice(0, -1);
  const arrivalTarget = arrivals[0]?.target;
  if (!arrivalTarget) return null;

  const everyArrivalIsSelfJoinByDeparture = arrivals.every((arrival) => {
    const actor = arrival?.actor ? normalizePubkey(arrival.actor) : null;
    const target = arrival?.target ? normalizePubkey(arrival.target) : null;
    return (
      arrival?.type === "member_joined" &&
      actor !== null &&
      target !== null &&
      actor === target &&
      target === departureActor
    );
  });
  if (!everyArrivalIsSelfJoinByDeparture) return null;

  return { type: "member_joined_then_left", target: arrivalTarget };
}
