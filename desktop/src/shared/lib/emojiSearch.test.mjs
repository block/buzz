import assert from "node:assert/strict";
import test from "node:test";

import {
  fuzzyStandardEmoji,
  normalizeShortcode,
  rankByShortcode,
  scoreShortcodeMatch,
} from "./emojiSearch.ts";

test("normalizeShortcode lowercases and strips separators", () => {
  assert.equal(normalizeShortcode(":Point_Up:"), "pointup");
  assert.equal(normalizeShortcode("man-woman-boy"), "manwomanboy");
  assert.equal(normalizeShortcode("thumbs up"), "thumbsup");
});

test("separator-only difference is an exact match (pointup == point_up)", () => {
  const m = scoreShortcodeMatch("pointup", "point_up");
  assert.ok(m);
  assert.equal(m.tier, 0); // both normalize to "pointup"
});

test("interior substring across `_` matches (ntup -> point_up)", () => {
  const m = scoreShortcodeMatch("ntup", "point_up");
  assert.ok(m);
  assert.equal(m.tier, 2); // substring, not a prefix
});

test("subsequence matches with omitted chars (pntup -> point_up)", () => {
  const m = scoreShortcodeMatch("pntup", "point_up");
  assert.ok(m);
  assert.equal(m.tier, 3); // subsequence
});

test("leading + interior omission still matches via subsequence (ointp)", () => {
  const m = scoreShortcodeMatch("ointp", "point_up");
  assert.ok(m);
  assert.equal(m.tier, 3);
});

test("exact and prefix rank above substring and subsequence", () => {
  assert.equal(scoreShortcodeMatch("pointup", "point_up").tier, 0); // exact
  assert.equal(scoreShortcodeMatch("point", "point_up").tier, 1); // prefix
  assert.equal(scoreShortcodeMatch("ntup", "point_up").tier, 2); // substring
  assert.equal(scoreShortcodeMatch("pntup", "point_up").tier, 3); // subsequence
});

test("no match returns null", () => {
  assert.equal(scoreShortcodeMatch("xyz", "point_up"), null);
  assert.equal(scoreShortcodeMatch("", "point_up"), null);
});

test("rankByShortcode orders exact > prefix > substring > subsequence", () => {
  const items = [
    { code: "point" }, // exact
    { code: "point_up" }, // -> "pointup": prefix (query is a prefix of it)
    { code: "endpoint" }, // -> "endpoint": substring
    { code: "pot_in_time" }, // -> "potintime": subsequence with gaps
    { code: "unrelated" }, // no match
  ];
  const ranked = rankByShortcode("point", items, (i) => i.code, 10).map(
    (i) => i.code,
  );
  assert.deepEqual(ranked, [
    "point", // exact
    "point_up", // prefix
    "endpoint", // substring
    "pot_in_time", // subsequence
  ]);
});

test("rankByShortcode respects the limit", () => {
  const items = ["smile", "smiley", "smiling", "smirk"].map((code) => ({
    code,
  }));
  const ranked = rankByShortcode("smi", items, (i) => i.code, 2);
  assert.equal(ranked.length, 2);
});

test("fuzzyStandardEmoji surfaces point_up for `pointup`", () => {
  const hits = fuzzyStandardEmoji("pointup", 8, new Set());
  const ids = hits.map((e) => e.id);
  assert.ok(ids.includes("point_up"), `expected point_up in ${ids.join(",")}`);
  assert.ok(hits.every((e) => e.native !== ""));
});

test("fuzzyStandardEmoji excludes already-shown ids", () => {
  const hits = fuzzyStandardEmoji("pointup", 8, new Set(["point_up"]));
  assert.ok(!hits.some((e) => e.id === "point_up"));
});

test("fuzzyStandardEmoji returns nothing when limit is non-positive", () => {
  assert.deepEqual(fuzzyStandardEmoji("pointup", 0, new Set()), []);
});
