// Rows younger than this get the entrance animation; anything older (opening
// a chat, reconnect backfill, history pagination) renders statically so
// existing conversation content doesn't cascade in.
const ENTRANCE_RECENCY_WINDOW_MS = 10_000;

export const MESSAGE_ENTRANCE_CLASS = "buzz-message-entrance";

export function isEntranceRecent(timestampMs: number, nowMs = Date.now()) {
  return (
    Number.isFinite(timestampMs) &&
    nowMs - timestampMs < ENTRANCE_RECENCY_WINDOW_MS
  );
}

/** Entrance class for an ISO-8601 timestamp (transcript items). */
export function entranceClassForIso(timestamp?: string | null) {
  if (!timestamp) {
    return undefined;
  }
  return isEntranceRecent(Date.parse(timestamp))
    ? MESSAGE_ENTRANCE_CLASS
    : undefined;
}

/** Entrance class for a Nostr `created_at` (seconds since epoch). */
export function entranceClassForCreatedAt(createdAtSeconds: number) {
  return isEntranceRecent(createdAtSeconds * 1_000)
    ? MESSAGE_ENTRANCE_CLASS
    : undefined;
}
