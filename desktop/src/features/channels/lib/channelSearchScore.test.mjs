import assert from "node:assert/strict";
import test from "node:test";

import { scoreChannelMatch, scoreChannelName } from "./channelSearchScore.ts";

test("scoreChannelName: empty query matches everything at top score", () => {
  assert.equal(scoreChannelName("release-notes", ""), 0);
});

test("scoreChannelName: exact name beats prefix beats word matches", () => {
  const exact = scoreChannelName("general", "general");
  const prefix = scoreChannelName("general-chat", "general");
  const wordExact = scoreChannelName("team-general", "general");
  assert.ok(exact < prefix, "exact should rank above prefix");
  assert.ok(prefix < wordExact, "prefix should rank above later-word match");
});

test("scoreChannelName: matches a later whole word", () => {
  // "notes" is the second word of "release-notes"
  assert.notEqual(scoreChannelName("release-notes", "notes"), null);
});

test("scoreChannelName: matches a later word prefix", () => {
  assert.notEqual(scoreChannelName("release-notes", "not"), null);
});

test("scoreChannelName: plain substring still matches", () => {
  assert.notEqual(scoreChannelName("release-notes", "ease"), null);
});

test("scoreChannelName: collapsing separators matches 'releasenotes'", () => {
  // The core pain point: dropping the dash should still find the channel.
  const score = scoreChannelName("release-notes", "releasenotes");
  assert.notEqual(score, null);
});

test("scoreChannelName: partial across the separator ('releasenot')", () => {
  assert.notEqual(scoreChannelName("release-notes", "releasenot"), null);
});

test("scoreChannelName: subsequence matches 'reln'", () => {
  // r-e-l...n appears in order inside "release-notes"
  assert.notEqual(scoreChannelName("release-notes", "reln"), null);
});

test("scoreChannelName: subsequence works across underscores and dots", () => {
  assert.notEqual(scoreChannelName("build_and_deploy", "bd"), null);
  assert.notEqual(scoreChannelName("v1.2.release", "vrel"), null);
});

test("scoreChannelName: single-char subsequence noise is rejected", () => {
  // A lone char that isn't a prefix/substring should NOT fuzzy-match, or every
  // channel containing that letter would show up.
  assert.equal(scoreChannelName("release-notes", "z"), null);
  // "x" doesn't appear at all
  assert.equal(scoreChannelName("release-notes", "x"), null);
});

test("scoreChannelName: unrelated query returns null", () => {
  assert.equal(scoreChannelName("release-notes", "budget"), null);
});

test("scoreChannelName: subsequence requires correct order", () => {
  // "sn" — 's' then 'n' — is NOT in order in "release-notes" (n comes... yes it
  // is: relea-s-e-n-otes). Use a genuinely out-of-order example instead.
  assert.equal(scoreChannelName("abc", "ca"), null);
});

test("scoreChannelName: better matches score lower than fuzzier ones", () => {
  const prefix = scoreChannelName("release-notes", "release");
  const collapsed = scoreChannelName("release-notes", "releasenotes");
  const subsequence = scoreChannelName("release-notes", "reln");
  assert.ok(prefix < collapsed, "prefix beats collapsed-separator match");
  assert.ok(collapsed < subsequence, "collapsed beats subsequence match");
});

test("scoreChannelMatch: name match outranks description match", () => {
  const nameHit = scoreChannelMatch(
    { name: "release-notes", description: "" },
    "release",
  );
  const descHit = scoreChannelMatch(
    { name: "random", description: "release coordination" },
    "release",
  );
  assert.ok(
    nameHit !== null && descHit !== null && nameHit < descHit,
    "a name match should rank above a description-only match",
  );
});

test("scoreChannelMatch: description only does plain substring, not fuzzy", () => {
  // "reln" should not fuzzy-match the description.
  assert.equal(
    scoreChannelMatch({ name: "random", description: "release notes" }, "reln"),
    null,
  );
  // But a real substring in the description matches.
  assert.notEqual(
    scoreChannelMatch(
      { name: "random", description: "release notes" },
      "notes",
    ),
    null,
  );
});

test("scoreChannelMatch: no match anywhere returns null", () => {
  assert.equal(
    scoreChannelMatch({ name: "general", description: "chat" }, "budget"),
    null,
  );
});

test("scoreChannelName: 'achannelname' finds 'a-channel-name'", () => {
  assert.notEqual(scoreChannelName("a-channel-name", "achannelname"), null);
});

test("scoreChannelName: dashed query still matches 'a-channel-name'", () => {
  assert.notEqual(scoreChannelName("a-channel-name", "a-channel-name"), null);
  assert.notEqual(scoreChannelName("a-channel-name", "a-channel"), null);
});

test("scoreChannelName: 'achannelname' finds snake_case 'a_channel_name'", () => {
  assert.notEqual(scoreChannelName("a_channel_name", "achannelname"), null);
});

test("scoreChannelName: 'achannelname' finds unicode-dash 'a\u2013channel\u2013name'", () => {
  assert.notEqual(
    scoreChannelName("a\u2013channel\u2013name", "achannelname"),
    null,
  );
});

test("scoreChannelName: dashed exact match ranks above collapsed-only match", () => {
  const dashed = scoreChannelName("a-channel-name", "a-channel-name");
  const collapsed = scoreChannelName("a-channel-name", "achannelname");
  assert.ok(dashed !== null && collapsed !== null && dashed < collapsed);
});

test("scoreChannelName: separators-only query does not match everything", () => {
  assert.equal(scoreChannelName("a-channel-name", "---"), null);
});
