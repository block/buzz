/**
 * Pure sanitizers for the `/meetings` route search params.
 *
 * Join and create are deliberately **not** symmetric. HiveTalk's URL-safe
 * room-name rule binds at registration only; rooms created on the HiveTalk
 * dashboard carry names that rule forbids. Kept out of the route file so they
 * can be unit-tested without a router (Phase 3 test policy: pure helpers only).
 */

import {
  MEETING_ROOM_NAME_MAX,
  MEETING_ROOM_NAME_MIN,
  normalizeMeetingRoomName,
} from "@/features/meetings/ui/meetingRoomName";

export type MeetingsRouteSearch = {
  room?: string;
  action?: "join" | "start";
};

/**
 * Sanitize a room name that is about to be **registered**: the same
 * normalization the start form applies, so a hand-edited or stale URL can't
 * push an unregistrable name into `register-room`.
 */
export function sanitizeRoomToCreate(raw: string): string | undefined {
  const normalized = normalizeMeetingRoomName(raw)
    .slice(0, MEETING_ROOM_NAME_MAX)
    .replace(/[-_]+$/, "");
  return normalized.length >= MEETING_ROOM_NAME_MIN ? normalized : undefined;
}

/**
 * Sanitize a room name that is about to be **joined**.
 *
 * Deliberately does not normalize: normalizing turned the dashboard-created
 * room `"Celestial  Solace"` into `celestial-solace`, which `get-token`
 * rejects with `403 room_not_registered` — a room the user owns and can see in
 * "My rooms" became unjoinable. Trim, bound the length, reject control
 * characters, and otherwise pass the name through exactly as HiveTalk gave it.
 */
export function sanitizeRoomToJoin(raw: string): string | undefined {
  const trimmed = raw.trim().slice(0, MEETING_ROOM_NAME_MAX);
  if (trimmed.length === 0) return undefined;
  for (let i = 0; i < trimmed.length; i += 1) {
    const code = trimmed.charCodeAt(i);
    if (code < 0x20 || code === 0x7f) return undefined;
  }
  return trimmed;
}

/** Validate the `/meetings` search params. */
export function validateMeetingsSearch(
  search: Record<string, unknown>,
): MeetingsRouteSearch {
  const action =
    search.action === "join" || search.action === "start"
      ? search.action
      : undefined;
  const raw = typeof search.room === "string" ? search.room : "";
  const room =
    action === "join" ? sanitizeRoomToJoin(raw) : sanitizeRoomToCreate(raw);
  return { room, action };
}
