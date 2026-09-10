import assert from "node:assert/strict";
import test from "node:test";
import {
  buildBwIssueRootTags,
  normalizeTemplateLines,
} from "./bwIssueCreation.ts";

const repo = `30617:${"b".repeat(64)}:repo`;

test("a BW issue root carries only a/subject — no recipient p tag", () => {
  const tags = buildBwIssueRootTags(repo, "  Title  ");
  assert.deepEqual(tags, [
    ["a", repo],
    ["subject", "Title"],
  ]);
  assert.ok(
    !tags.some((tag) => tag[0] === "p"),
    "a p tag would make the root permanently unenrollable (BW kind:1621 shape)",
  );
});

test("root creation rejects an empty title and an oversized title", () => {
  assert.throws(() => buildBwIssueRootTags(repo, "   "));
  assert.throws(() => buildBwIssueRootTags(repo, "x".repeat(257)));
});

test("root creation rejects a repo address that is not a kind:30617 coordinate", () => {
  assert.throws(() => buildBwIssueRootTags("not-a-repo", "Title"));
});

test("template lines are trimmed and blank lines are dropped", () => {
  assert.deepEqual(normalizeTemplateLines(["  first  ", "", "   ", "second"]), [
    "first",
    "second",
  ]);
});
