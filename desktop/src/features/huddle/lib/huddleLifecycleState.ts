import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_HUDDLE_ENDED,
  KIND_HUDDLE_PARTICIPANT_JOINED,
  KIND_HUDDLE_PARTICIPANT_LEFT,
  KIND_HUDDLE_STARTED,
} from "@/shared/constants/kinds";
import { HUDDLE_JOINABLE_WINDOW_SECONDS } from "./huddleCardState";

export type HuddleLifecycleState = {
  ended: boolean;
  participants: Set<string>;
  startCreatedAt: number | null;
  staleDeadlineMs: number | null;
};

type ReconstructHuddleOptions = {
  isCurrentHuddle?: boolean;
  nowMs?: number;
};

export function huddleEventChannelId(event: RelayEvent): string | null {
  try {
    const parsed = JSON.parse(event.content) as {
      ephemeral_channel_id?: unknown;
    };
    return typeof parsed.ephemeral_channel_id === "string"
      ? parsed.ephemeral_channel_id
      : null;
  } catch {
    return null;
  }
}

function lifecycleParticipant(event: RelayEvent): string | null {
  return (
    event.tags.find(
      (tag) => tag[0] === "p" && typeof tag[1] === "string",
    )?.[1] ??
    event.pubkey ??
    null
  );
}

/**
 * Reconstruct one huddle from its lifecycle events.
 *
 * An inferred huddle with no START event stays non-terminal because the
 * subscription window may have truncated an older participant JOIN. A retained
 * START makes an empty reconstructed participant set conclusive.
 */
export function reconstructHuddleState(
  events: Iterable<RelayEvent>,
  ephemeralChannelId: string,
  options: ReconstructHuddleOptions = {},
): HuddleLifecycleState {
  const sorted = [...events]
    .filter((event) => huddleEventChannelId(event) === ephemeralChannelId)
    .sort(
      (left, right) =>
        left.created_at - right.created_at ||
        left.kind - right.kind ||
        left.id.localeCompare(right.id),
    );
  let participants = new Set<string>();
  let explicitlyEnded = false;
  let startCreatedAt: number | null = null;
  let sawParticipantEventAfterStart = false;

  for (const event of sorted) {
    switch (event.kind) {
      case KIND_HUDDLE_STARTED:
        if (explicitlyEnded) break;
        startCreatedAt = event.created_at;
        participants = new Set(event.pubkey ? [event.pubkey] : []);
        sawParticipantEventAfterStart = false;
        break;
      case KIND_HUDDLE_PARTICIPANT_JOINED: {
        if (explicitlyEnded) break;
        if (startCreatedAt !== null) sawParticipantEventAfterStart = true;
        const pubkey = lifecycleParticipant(event);
        if (pubkey) participants.add(pubkey);
        break;
      }
      case KIND_HUDDLE_PARTICIPANT_LEFT: {
        if (explicitlyEnded) break;
        if (startCreatedAt !== null) sawParticipantEventAfterStart = true;
        const pubkey = lifecycleParticipant(event);
        if (pubkey) participants.delete(pubkey);
        break;
      }
      case KIND_HUDDLE_ENDED:
        explicitlyEnded = true;
        break;
    }
  }

  // An empty set is conclusive only when START is retained: without START,
  // the subscription's 100-event window may have truncated an older JOIN.
  const drained = startCreatedAt !== null && participants.size === 0;
  // START age is only a fallback for a huddle that never produced newer
  // lifecycle evidence. The relay TTL is renewable, so a later JOIN/LEFT or
  // the locally current huddle must not be expired from START time alone.
  const staleDeadlineMs =
    startCreatedAt !== null &&
    !sawParticipantEventAfterStart &&
    !options.isCurrentHuddle &&
    !explicitlyEnded &&
    !drained
      ? (startCreatedAt + HUDDLE_JOINABLE_WINDOW_SECONDS) * 1000 + 1
      : null;
  const nowMs = options.nowMs ?? Date.now();

  return {
    ended:
      explicitlyEnded ||
      drained ||
      (staleDeadlineMs !== null && nowMs >= staleDeadlineMs),
    participants,
    startCreatedAt,
    staleDeadlineMs,
  };
}

/** Delay until an unconfirmed START crosses the shared stale boundary. */
export function huddleStalenessDelayMs(
  staleDeadlineMs: number | null,
  nowMs = Date.now(),
): number | null {
  if (staleDeadlineMs === null) return null;
  return Math.max(0, staleDeadlineMs - nowMs);
}
