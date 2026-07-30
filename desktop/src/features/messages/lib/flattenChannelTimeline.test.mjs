import assert from "node:assert/strict";
import test from "node:test";

import {
  mergeFlattenTimelineReplies,
  flattenTimelineRootIds,
} from "./flattenChannelTimeline.ts";
import { shouldFlattenChannelTimeline } from "./threading.ts";

test("shouldFlattenChannelTimeline is true for private rooms and DMs only", () => {
  assert.equal(
    shouldFlattenChannelTimeline({
      channelType: "stream",
      visibility: "private",
    }),
    true,
  );
  assert.equal(
    shouldFlattenChannelTimeline({ channelType: "dm", visibility: "private" }),
    true,
  );
  assert.equal(
    shouldFlattenChannelTimeline({ channelType: "stream", visibility: "open" }),
    false,
  );
  assert.equal(shouldFlattenChannelTimeline(null), false);
});

test("mergeFlattenTimelineReplies preserves reply tags and dedupes by id", () => {
  const root = {
    id: "root",
    pubkey: "a",
    created_at: 1,
    kind: 9,
    tags: [["h", "channel"]],
    content: "root",
    sig: "",
  };
  const reply = {
    id: "reply",
    pubkey: "b",
    created_at: 2,
    kind: 9,
    tags: [
      ["h", "channel"],
      ["e", "root", "", "reply"],
    ],
    content: "reply",
    sig: "",
  };

  const merged = mergeFlattenTimelineReplies([root], [reply, reply]);
  assert.deepEqual(
    merged.map((event) => event.id),
    ["root", "reply"],
  );
  assert.deepEqual(merged[1].tags, reply.tags);
});

test("flattenTimelineRootIds reads page and live summary roots", () => {
  const store = {
    pages: [
      {
        startCursor: null,
        rows: [
          {
            event: {
              id: "root-a",
              pubkey: "a",
              created_at: 1,
              kind: 9,
              tags: [],
              content: "",
              sig: "",
            },
            thread: {
              replyCount: 1,
              descendantCount: 1,
              lastReplyAt: 2,
              participantPubkeys: ["b"],
            },
          },
        ],
        aux: [],
        hasMore: false,
        nextCursor: null,
      },
    ],
    liveOverlay: [],
    liveAux: [],
    liveSummaries: {
      "root-b": {
        summary: {
          replyCount: 2,
          descendantCount: 2,
          lastReplyAt: 3,
          participantPubkeys: ["c"],
        },
        createdAt: 3,
      },
    },
  };

  assert.deepEqual(flattenTimelineRootIds(store).sort(), ["root-a", "root-b"]);
});
