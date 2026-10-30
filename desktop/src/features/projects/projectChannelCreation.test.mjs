import assert from "node:assert/strict";
import { test } from "node:test";

import { buildProjectRelatedChannelPatchTemplate } from "./projectChannelCreation.ts";
import {
  MAX_PROJECT_RELATED_CHANNELS,
  PROJECT_RELATED_CHANNEL_TAG,
} from "./projectModels.ts";

const OWNER = "a".repeat(64);
const OTHER = "b".repeat(64);
const CHANNEL_A = "11111111-1111-4111-8111-111111111111";
const CHANNEL_B = "22222222-2222-4222-8222-222222222222";

function liveHead(tags = []) {
  return {
    content: "",
    created_at: 100,
    id: "project-head",
    kind: 30621,
    pubkey: OWNER,
    sig: "sig",
    tags: [
      ["d", "sprout"],
      ["name", "Sprout"],
      ["buzz-channel", CHANNEL_A],
      ...tags,
    ],
  };
}

test("appends a related channel tag and preserves the live head", () => {
  const patched = buildProjectRelatedChannelPatchTemplate({
    channelId: CHANNEL_B,
    liveHead: liveHead([["description", "A project"]]),
    ownerPubkey: OWNER,
  });

  assert.equal(patched.alreadyBound, false);
  assert.deepEqual(patched.project.tags, [
    ["d", "sprout"],
    ["name", "Sprout"],
    ["buzz-channel", CHANNEL_A],
    ["description", "A project"],
    [PROJECT_RELATED_CHANNEL_TAG, CHANNEL_B],
  ]);
});

test("is idempotent when the related channel is already tagged", () => {
  const patched = buildProjectRelatedChannelPatchTemplate({
    channelId: CHANNEL_B,
    liveHead: liveHead([[PROJECT_RELATED_CHANNEL_TAG, CHANNEL_B]]),
    ownerPubkey: OWNER,
  });

  assert.equal(patched.alreadyBound, true);
  assert.equal(
    patched.project.tags.filter((tag) => tag[0] === PROJECT_RELATED_CHANNEL_TAG)
      .length,
    1,
  );
});

test("refuses to bind the home channel as a related channel", () => {
  assert.throws(
    () =>
      buildProjectRelatedChannelPatchTemplate({
        channelId: CHANNEL_A,
        liveHead: liveHead(),
        ownerPubkey: OWNER,
      }),
    /already this project's home/,
  );
});

test("only the project owner can add related channels", () => {
  assert.throws(
    () =>
      buildProjectRelatedChannelPatchTemplate({
        channelId: CHANNEL_B,
        liveHead: liveHead(),
        ownerPubkey: OTHER,
      }),
    /Only the project owner/,
  );
});

test("caps extra related channels", () => {
  const tags = Array.from(
    { length: MAX_PROJECT_RELATED_CHANNELS },
    (_, index) => {
      const suffix = String(index + 1).padStart(12, "0");
      return [PROJECT_RELATED_CHANNEL_TAG, `33333333-3333-4333-8333-${suffix}`];
    },
  );
  assert.throws(
    () =>
      buildProjectRelatedChannelPatchTemplate({
        channelId: CHANNEL_B,
        liveHead: liveHead(tags),
        ownerPubkey: OWNER,
      }),
    /extra channels/,
  );
});
