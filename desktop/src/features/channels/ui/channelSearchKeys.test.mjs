import assert from "node:assert/strict";
import test from "node:test";

import { buildMessageRouteTargetClearPatch } from "./channelSearchKeys.ts";

test("clearing a message route removes its view mode and thread anchor", () => {
  assert.deepEqual(buildMessageRouteTargetClearPatch(), {
    messageId: null,
    messageView: null,
    threadRootId: null,
  });
});
