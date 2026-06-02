import assert from "node:assert/strict";
import test from "node:test";

import {
  getContextMessageDepth,
  getReactionTargetId,
  matchesInboxFilter,
} from "./inboxViewHelpers.ts";

// --- matchesInboxFilter ---

test("matchesInboxFilter returns true for the 'all' filter regardless of categories", () => {
  assert.equal(matchesInboxFilter({ categories: [] }, "all"), true);
  assert.equal(matchesInboxFilter({ categories: ["mentions"] }, "all"), true);
});

test("matchesInboxFilter matches when the category is present", () => {
  assert.equal(
    matchesInboxFilter({ categories: ["mentions", "activity"] }, "mentions"),
    true,
  );
});

test("matchesInboxFilter is false when the category is absent", () => {
  assert.equal(
    matchesInboxFilter({ categories: ["activity"] }, "mentions"),
    false,
  );
  assert.equal(matchesInboxFilter({ categories: [] }, "mentions"), false);
});

// --- getReactionTargetId ---

test("getReactionTargetId returns the last e-tag target id", () => {
  const tags = [
    ["e", "first"],
    ["p", "somebody"],
    ["e", "second"],
  ];
  assert.equal(getReactionTargetId(tags), "second");
});

test("getReactionTargetId returns null when there is no e-tag", () => {
  assert.equal(getReactionTargetId([["p", "somebody"]]), null);
  assert.equal(getReactionTargetId([]), null);
});

test("getReactionTargetId ignores e-tags with a missing/non-string id", () => {
  // Trailing malformed e-tag should be skipped in favor of the valid one.
  const tags = [["e", "valid"], ["e"]];
  assert.equal(getReactionTargetId(tags), "valid");
});

// --- getContextMessageDepth ---

function event(id, parentId) {
  // A "reply" e-tag is how getThreadReference resolves a parent.
  const tags = parentId ? [["e", parentId, "", "reply"]] : [];
  return {
    id,
    pubkey: "x",
    created_at: 0,
    kind: 9,
    tags,
    content: "",
    sig: "",
  };
}

test("getContextMessageDepth is 0 for a root message", () => {
  const root = event("root", null);
  const map = new Map([[root.id, root]]);
  assert.equal(getContextMessageDepth(root, map), 0);
});

test("getContextMessageDepth counts ancestors present in the map", () => {
  const root = event("root", null);
  const mid = event("mid", "root");
  const leaf = event("leaf", "mid");
  const map = new Map([
    [root.id, root],
    [mid.id, mid],
    [leaf.id, leaf],
  ]);
  assert.equal(getContextMessageDepth(leaf, map), 2);
  assert.equal(getContextMessageDepth(mid, map), 1);
});

test("getContextMessageDepth stops when a parent is missing from the map", () => {
  // leaf -> mid (present) -> absent root. Depth counts only the present hop.
  const mid = event("mid", "absent-root");
  const leaf = event("leaf", "mid");
  const map = new Map([
    [mid.id, mid],
    [leaf.id, leaf],
  ]);
  assert.equal(getContextMessageDepth(leaf, map), 1);
});

test("getContextMessageDepth does not loop forever on a cycle", () => {
  // a -> b -> a. The `seen` set must terminate the walk.
  const a = event("a", "b");
  const b = event("b", "a");
  const map = new Map([
    [a.id, a],
    [b.id, b],
  ]);
  // From a: hop to b (depth 1); b's parent is a, already seen -> stop.
  assert.equal(getContextMessageDepth(a, map), 1);
});
