import assert from "node:assert/strict";
import test from "node:test";

import { appearanceCommunityLabel } from "./appearanceScopeCopy.ts";

test("labels the active community by name", () => {
  assert.equal(appearanceCommunityLabel("Block Builders"), "Block Builders");
});

test("trims surrounding whitespace from the community name", () => {
  assert.equal(appearanceCommunityLabel("  Buzz HQ  "), "Buzz HQ");
});

test("falls back to a generic label when no community is active", () => {
  assert.equal(appearanceCommunityLabel(null), "this community");
  assert.equal(appearanceCommunityLabel(undefined), "this community");
  assert.equal(appearanceCommunityLabel(""), "this community");
  assert.equal(appearanceCommunityLabel("   "), "this community");
});
