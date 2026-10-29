import assert from "node:assert/strict";
import test from "node:test";

import { getTopLevelInboxUnreadOverrideIds } from "./useChannelOpenReadState.ts";

test("opening a channel clears only its top-level Inbox overrides", () => {
  assert.deepEqual(
    getTopLevelInboxUnreadOverrideIds(
      [
        { id: "top-level", channelId: "general", tags: [] },
        {
          id: "thread-reply",
          channelId: "general",
          tags: [
            ["e", "root", "", "root"],
            ["e", "parent", "", "reply"],
          ],
        },
        { id: "other-channel", channelId: "random", tags: [] },
      ],
      "general",
    ),
    ["top-level"],
  );
});
