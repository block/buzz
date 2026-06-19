import assert from "node:assert/strict";
import test from "node:test";

import { shouldRouteChannelUnreadEvent } from "./useLiveChannelUpdates.ts";

test("main-channel messages route to channel unread tracking", () => {
  assert.equal(shouldRouteChannelUnreadEvent(undefined, false), true);
});

test("non-DM thread replies do not route to channel unread tracking", () => {
  assert.equal(
    shouldRouteChannelUnreadEvent({ channelType: "stream" }, true),
    false,
  );
});

test("DM thread replies route to channel unread tracking", () => {
  assert.equal(
    shouldRouteChannelUnreadEvent({ channelType: "dm" }, true),
    true,
  );
});
