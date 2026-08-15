import assert from "node:assert/strict";
import { test } from "node:test";

import {
  buildFileVersionChains,
  buildLatestVersionIndex,
} from "./fileVersionChains.mjs";

/** Minimal ChannelFileEntry stand-in. `at` orders uploads. */
function file(eventId, { supersedes = null, at = 0, filename = null } = {}) {
  return {
    eventId,
    supersedes,
    supersededBy: null,
    uploadedAt: at,
    filename: filename ?? `${eventId}.pdf`,
  };
}

test("unversioned files each come back as their own chain", () => {
  const chains = buildFileVersionChains([
    file("a", { at: 2 }),
    file("b", { at: 1 }),
  ]);
  assert.equal(chains.length, 2);
  assert.deepEqual(
    chains.map((c) => c.latest.eventId),
    ["a", "b"],
  );
  assert.ok(chains.every((c) => c.older.length === 0));
});

test("a three-version chain collapses under its newest file", () => {
  // v1 <- v2 <- v3
  const chains = buildFileVersionChains([
    file("v1", { at: 1 }),
    file("v2", { at: 2, supersedes: "v1" }),
    file("v3", { at: 3, supersedes: "v2" }),
  ]);
  assert.equal(chains.length, 1);
  assert.equal(chains[0].latest.eventId, "v3");
  assert.deepEqual(
    chains[0].older.map((f) => f.eventId),
    ["v2", "v1"],
  );
});

test("every version resolves straight to the head, not one step", () => {
  const latest = buildLatestVersionIndex([
    file("v1", { at: 1 }),
    file("v2", { at: 2, supersedes: "v1" }),
    file("v3", { at: 3, supersedes: "v2" }),
  ]);
  assert.equal(latest.get("v1"), "v3");
  assert.equal(latest.get("v2"), "v3");
  assert.equal(latest.get("v3"), "v3");
});

test("chains are ordered by their newest file", () => {
  const chains = buildFileVersionChains([
    file("old1", { at: 1 }),
    file("old2", { at: 2, supersedes: "old1" }),
    file("solo", { at: 5 }),
  ]);
  assert.deepEqual(
    chains.map((c) => c.latest.eventId),
    ["solo", "old2"],
  );
});

test("a link to a file we do not have is ignored, not fatal", () => {
  // The parent was deleted or lies beyond the fetched pages.
  const chains = buildFileVersionChains([
    file("v2", { at: 2, supersedes: "missing-v1" }),
  ]);
  assert.equal(chains.length, 1);
  assert.equal(chains[0].latest.eventId, "v2");
  assert.deepEqual(chains[0].older, []);
});

test("a cycle terminates and loses no files", () => {
  // a -> b -> a. Malformed, but must not hang or drop anything.
  const chains = buildFileVersionChains([
    file("a", { at: 1, supersedes: "b" }),
    file("b", { at: 2, supersedes: "a" }),
  ]);
  const surfaced = new Set(
    chains.flatMap((c) => [c.latest.eventId, ...c.older.map((f) => f.eventId)]),
  );
  assert.ok(surfaced.has("a"));
  assert.ok(surfaced.has("b"));
});

test("resolveLatestEventId terminates on a cycle", () => {
  const latest = buildLatestVersionIndex([
    file("a", { at: 1, supersedes: "b" }),
    file("b", { at: 2, supersedes: "a" }),
  ]);
  // Exact head is unknowable in a cycle; the contract is that it answers.
  assert.ok(latest.get("a"));
  assert.ok(latest.get("b"));
});

test("a fork keeps both files visible", () => {
  // Two people independently uploaded a newer version of v1.
  const chains = buildFileVersionChains([
    file("v1", { at: 1 }),
    file("fork-a", { at: 2, supersedes: "v1" }),
    file("fork-b", { at: 3, supersedes: "v1" }),
  ]);
  const surfaced = new Set(
    chains.flatMap((c) => [c.latest.eventId, ...c.older.map((f) => f.eventId)]),
  );
  // The later upload wins the parent; the other stands alone rather than
  // disappearing under a sibling it has no relationship to.
  assert.ok(surfaced.has("v1"));
  assert.ok(surfaced.has("fork-a"));
  assert.ok(surfaced.has("fork-b"));
  const winner = chains.find((c) => c.latest.eventId === "fork-b");
  assert.deepEqual(
    winner.older.map((f) => f.eventId),
    ["v1"],
  );
});

test("a file cannot supersede itself", () => {
  const chains = buildFileVersionChains([
    file("self", { at: 1, supersedes: "self" }),
  ]);
  assert.equal(chains.length, 1);
  assert.equal(chains[0].latest.eventId, "self");
  assert.deepEqual(chains[0].older, []);
});

test("empty and nullish input are handled", () => {
  assert.deepEqual(buildFileVersionChains([]), []);
  assert.deepEqual(buildFileVersionChains(null), []);
  assert.equal(buildLatestVersionIndex(null).size, 0);
});
