/**
 * Room-name normalize + validate for the Meetings start form.
 *
 * HiveTalk room-name rule (plan §3 "Room facts"): URL-safe — letters/digits in
 * any script, plus `-` and `_`. No length bound is documented upstream; we cap
 * at a sane 3–64 so the UI rejects obvious junk before a round-trip.
 *
 * Pure: no I/O, no React. Tested by `meetingRoomName.test.mjs`.
 */

export const MEETING_ROOM_NAME_MIN = 3;
export const MEETING_ROOM_NAME_MAX = 64;

export type MeetingRoomNameResult =
  | { ok: true; value: string }
  | { ok: false; reason: string };

/**
 * Best-effort normalization: trim, fold internal whitespace to `-`, lowercase
 * (ASCII only — non-ASCII letters are kept verbatim), drop any character that
 * is not a Unicode letter/number/`-`/`_`, and collapse repeated separators.
 */
export function normalizeMeetingRoomName(input: string): string {
  return input
    .trim()
    .toLowerCase()
    .replace(/\s+/g, "-")
    .replace(/[^\p{L}\p{N}_-]+/gu, "")
    .replace(/-{2,}/g, "-")
    .replace(/^[-_]+|[-_]+$/g, "");
}

/** Normalize then bounds-check. Returns the cleaned name or a reason string. */
export function validateMeetingRoomName(input: string): MeetingRoomNameResult {
  const value = normalizeMeetingRoomName(input);
  if (value.length === 0) {
    return {
      ok: false,
      reason: "Use letters, numbers, dashes or underscores.",
    };
  }
  if (value.length < MEETING_ROOM_NAME_MIN) {
    return {
      ok: false,
      reason: `Room name needs at least ${MEETING_ROOM_NAME_MIN} characters.`,
    };
  }
  if (value.length > MEETING_ROOM_NAME_MAX) {
    return {
      ok: false,
      reason: `Room name can be at most ${MEETING_ROOM_NAME_MAX} characters.`,
    };
  }
  return { ok: true, value };
}
