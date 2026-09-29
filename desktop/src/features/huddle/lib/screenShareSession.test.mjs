import assert from "node:assert/strict";
import test from "node:test";

import {
  isShareBlockedByOther,
  livekitRoomName,
} from "./screenSharePolicy.ts";

test("livekitRoomName matches relay format", () => {
  assert.equal(
    livekitRoomName("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"),
    "huddle-aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
  );
});

test("isShareBlockedByOther allows empty sharer", () => {
  assert.equal(
    isShareBlockedByOther({ selfPubkey: "aa", currentSharer: null }),
    false,
  );
});

test("isShareBlockedByOther blocks other sharer", () => {
  assert.equal(
    isShareBlockedByOther({ selfPubkey: "aa", currentSharer: "bb" }),
    true,
  );
});

test("isShareBlockedByOther allows self sharer", () => {
  assert.equal(
    isShareBlockedByOther({ selfPubkey: "Aa", currentSharer: "aa" }),
    false,
  );
});
