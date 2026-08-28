/**
 * Pure helpers for the channel -> Meetings deep link.
 *
 * The channel "Start meeting" button routes to
 * `/meetings?room=<derived>&action=start`. The room name is derived
 * deterministically from the channel so everyone in a given channel lands on the
 * same room without typing anything.
 *
 * Pure: no I/O, no React. Tested by `meetingsDeepLink.test.mjs`.
 */

import {
  MEETING_ROOM_NAME_MAX,
  normalizeMeetingRoomName,
} from "@/features/meetings/ui/meetingRoomName";

export type MeetingsDeepLinkAction = "join" | "start";

export type MeetingsDeepLinkSearch = {
  room?: string;
  action?: MeetingsDeepLinkAction;
};

const CHANNEL_ID_SUFFIX_LENGTH = 8;

/** Short, stable suffix from the channel id so two channels whose names
 * normalize to the same value don't collide on one room. */
function channelIdSuffix(channelId: string): string {
  const cleaned = channelId.replace(/[^a-z0-9]/gi, "").toLowerCase();
  return cleaned.slice(0, CHANNEL_ID_SUFFIX_LENGTH) || "channel";
}

/**
 * Deterministic room name for a channel: normalized channel name plus a short
 * id-derived suffix, clamped to the HiveTalk room-name bound. Falls back to a
 * pure id-derived name when the channel name normalizes to nothing.
 */
export function deriveChannelMeetingRoomName(input: {
  channelId: string;
  channelName: string;
}): string {
  const suffix = channelIdSuffix(input.channelId);
  const base = normalizeMeetingRoomName(input.channelName);
  if (base.length === 0) {
    return `channel-${suffix}`;
  }
  const maxBase = MEETING_ROOM_NAME_MAX - (suffix.length + 1);
  const trimmedBase = base.slice(0, maxBase).replace(/[-_]+$/, "");
  return `${trimmedBase || "channel"}-${suffix}`;
}

/** Build the `/meetings` search params for a channel "Start meeting" action. */
export function buildChannelMeetingSearch(input: {
  channelId: string;
  channelName: string;
}): Required<Pick<MeetingsDeepLinkSearch, "room" | "action">> {
  return {
    room: deriveChannelMeetingRoomName(input),
    action: "start",
  };
}
