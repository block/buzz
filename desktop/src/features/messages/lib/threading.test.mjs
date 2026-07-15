import assert from "node:assert/strict";
import test from "node:test";

import { buildReplyTags } from "./threading.ts";

const CHANNEL_ID = "channel-1";
const AUTHOR = "author-pubkey";

function hasBroadcastTag(tags) {
  return tags.some(
    (tag) => tag.length >= 2 && tag[0] === "broadcast" && tag[1] === "1",
  );
}

test("buildReplyTags omits the broadcast tag by default", () => {
  const tags = buildReplyTags(CHANNEL_ID, AUTHOR, "root", "root");

  assert.equal(hasBroadcastTag(tags), false);
});

test("buildReplyTags adds the NIP-CW broadcast tag when requested", () => {
  const tags = buildReplyTags(CHANNEL_ID, AUTHOR, "root", "root", [], true);

  assert.equal(hasBroadcastTag(tags), true);
  // Thread e-tags must stay last so relay-side parsing is unaffected.
  assert.deepEqual(tags.at(-1), ["e", "root", "", "reply"]);
});

test("buildReplyTags keeps depth-1 reply shape with broadcast enabled", () => {
  const tags = buildReplyTags(CHANNEL_ID, AUTHOR, "root", "root", [], true);

  assert.deepEqual(tags, [
    ["p", AUTHOR],
    ["h", CHANNEL_ID],
    ["broadcast", "1"],
    ["e", "root", "", "reply"],
  ]);
});

test("buildReplyTags supports broadcast on nested replies (root + reply e-tags)", () => {
  // The UI only offers broadcast for direct replies to the thread head, but
  // the tag builder itself stays orthogonal to that policy.
  const tags = buildReplyTags(CHANNEL_ID, AUTHOR, "parent", "root", [], true);

  assert.equal(hasBroadcastTag(tags), true);
  assert.deepEqual(tags.slice(-2), [
    ["e", "root", "", "root"],
    ["e", "parent", "", "reply"],
  ]);
});
