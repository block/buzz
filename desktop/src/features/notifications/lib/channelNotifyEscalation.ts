import { isSelfPresenceOnline } from "@/features/presence/lib/selfPresence";
import {
  NOTIFY_TAG,
  isNotifyMode,
  type NotifyMode,
} from "@/shared/constants/notify";
import type { RelayEvent } from "@/shared/api/types";

/**
 * How far an `@here` event may be from "now" and still escalate.
 *
 * `@here` is live-only (NIP-CM): a reader who was offline when it was sent, or
 * who scrolls it into view later, gets nothing. 120s absorbs clock skew and
 * relay latency without turning it into a persistent mention.
 */
export const HERE_FRESHNESS_WINDOW_SECONDS = 120;

/** Observation-time inputs for the `@here` liveness check. */
export type ChannelNotifyContext = {
  /** Whether the reader's own presence reads as online. */
  selfOnline?: boolean;
  /** Observation time, in seconds since the epoch. */
  nowSeconds?: number;
};

/** Notify mode carried by an event's tags, or null when it carries none. */
export function notifyModeForTags(
  tags: readonly string[][],
): NotifyMode | null {
  for (const tag of tags) {
    const mode = tag[0] === NOTIFY_TAG ? tag[1] : undefined;
    if (mode !== undefined && isNotifyMode(mode)) {
      return mode;
    }
  }
  return null;
}

/**
 * Whether a channel-wide mention escalates this event to mention tier for the
 * reader.
 *
 * `@channel` always does; `@here` only while the reader is online and the
 * event is fresh. Authors never escalate their own messages. Mute suppression
 * is NOT decided here — callers apply it first (see `shouldNotifyForEvent`).
 */
export function channelNotifyEscalates(
  event: RelayEvent,
  currentPubkey: string,
  context: ChannelNotifyContext = {},
): boolean {
  if (currentPubkey.length === 0) {
    return false;
  }

  const mode = notifyModeForTags(event.tags);
  if (mode === null) {
    return false;
  }

  if (event.pubkey.toLowerCase() === currentPubkey.toLowerCase()) {
    return false;
  }

  if (mode === "channel") {
    return true;
  }

  if (!(context.selfOnline ?? isSelfPresenceOnline())) {
    return false;
  }

  const nowSeconds = context.nowSeconds ?? Math.floor(Date.now() / 1000);
  return (
    Math.abs(nowSeconds - event.created_at) <= HERE_FRESHNESS_WINDOW_SECONDS
  );
}
