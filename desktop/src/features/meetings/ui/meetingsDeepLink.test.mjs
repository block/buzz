import assert from "node:assert/strict";
import test from "node:test";

import { MEETING_ROOM_NAME_MAX } from "./meetingRoomName.ts";
import {
  buildChannelMeetingSearch,
  deriveChannelMeetingRoomName,
} from "./meetingsDeepLink.ts";

test("derive: normalized channel name plus id suffix", () => {
  assert.equal(
    deriveChannelMeetingRoomName({
      channelId: "75adcca5-92fa-4f2d-855d-42abefe7e8e8",
      channelName: "Buzz Hive Int",
    }),
    "buzz-hive-int-75adcca5",
  );
});

test("derive: is deterministic for the same channel", () => {
  const input = { channelId: "abc-123-def", channelName: "Weekly Sync" };
  assert.equal(
    deriveChannelMeetingRoomName(input),
    deriveChannelMeetingRoomName(input),
  );
});

test("derive: falls back to an id-only name when the name is all punctuation", () => {
  assert.equal(
    deriveChannelMeetingRoomName({
      channelId: "ffff0000-1111",
      channelName: "!!!",
    }),
    "channel-ffff0000",
  );
});

test("derive: handles a channel id with no alphanumerics", () => {
  assert.equal(
    deriveChannelMeetingRoomName({ channelId: "----", channelName: "Room" }),
    "room-channel",
  );
});

test("derive: clamps to the HiveTalk room-name bound", () => {
  const name = deriveChannelMeetingRoomName({
    channelId: "0123456789abcdef",
    channelName: "x".repeat(200),
  });
  assert.ok(name.length <= MEETING_ROOM_NAME_MAX);
  assert.ok(name.endsWith("-01234567"));
});

test("derive: no doubled separator between base and suffix", () => {
  const name = deriveChannelMeetingRoomName({
    channelId: "aaaaaaaa",
    channelName: "trailing dash -",
  });
  assert.equal(name, "trailing-dash-aaaaaaaa");
});

test("buildChannelMeetingSearch: start action + derived room", () => {
  assert.deepEqual(
    buildChannelMeetingSearch({
      channelId: "abc12345",
      channelName: "Design Review",
    }),
    { room: "design-review-abc12345", action: "start" },
  );
});
