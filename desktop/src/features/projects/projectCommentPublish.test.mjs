import assert from "node:assert/strict";
import test from "node:test";

import { KIND_STREAM_MESSAGE, KIND_TEXT_NOTE } from "@/shared/constants/kinds";
import { projectCommentKindAndChannelTags } from "./projectCommentPublish.mjs";

test("plain project comments stay kind:1 without a channel tag", () => {
  const result = projectCommentKindAndChannelTags(
    {
      owner: "a".repeat(64),
      projectChannelId: "channel-1",
      repoAddress: "30617:owner:repo",
    },
    [],
  );
  assert.equal(result.kind, KIND_TEXT_NOTE);
  assert.deepEqual(result.channelTags, []);
});

test("agent mentions publish as kind:9 with the project channel h tag", () => {
  const result = projectCommentKindAndChannelTags(
    {
      owner: "a".repeat(64),
      projectChannelId: "channel-1",
      repoAddress: "30617:owner:repo",
    },
    ["b".repeat(64)],
  );
  assert.equal(result.kind, KIND_STREAM_MESSAGE);
  assert.deepEqual(result.channelTags, [["h", "channel-1"]]);
});

test("agent mentions without a project channel fail closed", () => {
  assert.throws(
    () =>
      projectCommentKindAndChannelTags(
        {
          owner: "a".repeat(64),
          projectChannelId: null,
          repoAddress: "30617:owner:repo",
        },
        ["b".repeat(64)],
      ),
    /discussion channel/i,
  );
});
