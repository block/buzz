import assert from "node:assert/strict";
import test from "node:test";

import { collapseSeparators, isSubsequence } from "./identifierMatch.ts";

test("collapseSeparators strips dashes, underscores, dots, slashes, spaces", () => {
  assert.equal(collapseSeparators("a-channel-name"), "achannelname");
  assert.equal(collapseSeparators("a_channel name"), "achannelname");
  assert.equal(collapseSeparators("a.channel/name"), "achannelname");
});

test("collapseSeparators strips unicode dashes", () => {
  assert.equal(collapseSeparators("a\u2013channel\u2014name"), "achannelname");
  assert.equal(collapseSeparators("a\u2010channel\u2212name"), "achannelname");
});

test("collapseSeparators of a separators-only string is empty", () => {
  assert.equal(collapseSeparators("-- __ .."), "");
});

test("isSubsequence matches in-order chars", () => {
  assert.equal(isSubsequence("acn", "a-channel-name"), true);
  assert.equal(isSubsequence("nca", "a-channel-name"), false);
});
