/**
 * Request bodies for HiveTalk's moderation endpoints.
 *
 * Pinned to HiveTalk's `openapi.yaml` (`components.schemas.ParticipantAction`
 * and `RoomToggle`; archived at `RESEARCH/HIVETALK_OPENAPI.yaml` in the
 * Buzz_Cordinator workspace). The API is deliberately **mixed-case**: the
 * registry endpoints are snake_case (`/api/register-room` takes `room_name`
 * and answers `400 room_name is required` to anything else), while these
 * moderation endpoints are LiveKit-backed and camelCase. Do not "normalize"
 * one to match the other.
 *
 * These live outside `CallControlBar` so the wire shape can be asserted
 * without a DOM: the component builds no payload of its own.
 */

/** `ParticipantAction` — `/api/kick-user`, `/api/mute-user`. */
export function participantActionPayload(
  roomName: string,
  participantIdentity: string,
): { roomName: string; participantIdentity: string } {
  return { roomName, participantIdentity };
}

/** `RoomToggle` — `/api/room/notify-lock`, `/api/room/mute-on-join`. */
export function roomTogglePayload(
  roomName: string,
  enabled: boolean,
): { roomName: string; enabled: boolean } {
  return { roomName, enabled };
}
