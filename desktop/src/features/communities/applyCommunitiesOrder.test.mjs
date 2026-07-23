/**
 * Unit tests for applyCommunitiesOrder — the pure permutation helper that
 * drives community-rail drag-to-reorder.
 */
import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { applyCommunitiesOrder } from "./useCommunities.tsx";

const A = {
  id: "ws-a",
  name: "Alpha",
  relayUrl: "wss://a.example.com",
  addedAt: "2024-01-01",
};
const B = {
  id: "ws-b",
  name: "Bravo",
  relayUrl: "wss://b.example.com",
  addedAt: "2024-01-02",
};
const C = {
  id: "ws-c",
  name: "Charlie",
  relayUrl: "wss://c.example.com",
  addedAt: "2024-01-03",
};

describe("applyCommunitiesOrder", () => {
  it("reorders communities to match orderedIds", () => {
    const result = applyCommunitiesOrder([A, B, C], ["ws-c", "ws-a", "ws-b"]);
    assert.deepEqual(
      result.map((c) => c.id),
      ["ws-c", "ws-a", "ws-b"],
    );
  });

  it("returns same order when orderedIds matches current order", () => {
    const result = applyCommunitiesOrder([A, B, C], ["ws-a", "ws-b", "ws-c"]);
    assert.deepEqual(
      result.map((c) => c.id),
      ["ws-a", "ws-b", "ws-c"],
    );
  });

  it("appends communities not mentioned in orderedIds at the end in original relative order", () => {
    // C was added after drag — not in orderedIds — should tail-append
    const result = applyCommunitiesOrder([A, B, C], ["ws-b", "ws-a"]);
    assert.deepEqual(
      result.map((c) => c.id),
      ["ws-b", "ws-a", "ws-c"],
    );
  });

  it("handles orderedIds that contain stale IDs not present in communities", () => {
    // "ws-gone" is a stale id — silent skip, no crash
    const result = applyCommunitiesOrder([A, B], ["ws-b", "ws-gone", "ws-a"]);
    assert.deepEqual(
      result.map((c) => c.id),
      ["ws-b", "ws-a"],
    );
  });

  it("returns the full list when orderedIds is empty — original order preserved", () => {
    const result = applyCommunitiesOrder([A, B, C], []);
    assert.deepEqual(
      result.map((c) => c.id),
      ["ws-a", "ws-b", "ws-c"],
    );
  });

  it("handles a single-element list (no-op reorder)", () => {
    const result = applyCommunitiesOrder([A], ["ws-a"]);
    assert.deepEqual(
      result.map((c) => c.id),
      ["ws-a"],
    );
  });

  it("handles an empty communities list", () => {
    const result = applyCommunitiesOrder([], ["ws-a", "ws-b"]);
    assert.deepEqual(result, []);
  });

  it("does not mutate the original array", () => {
    const original = [A, B, C];
    applyCommunitiesOrder(original, ["ws-c", "ws-a", "ws-b"]);
    assert.deepEqual(
      original.map((c) => c.id),
      ["ws-a", "ws-b", "ws-c"],
    );
  });

  it("preserves object identity of each community (no clone)", () => {
    const result = applyCommunitiesOrder([A, B, C], ["ws-c", "ws-b", "ws-a"]);
    assert.equal(result[0], C);
    assert.equal(result[1], B);
    assert.equal(result[2], A);
  });

  it("handles duplicate IDs in orderedIds — first occurrence wins", () => {
    // Defensive: dnd-kit should never produce duplicates, but guard anyway.
    const result = applyCommunitiesOrder([A, B, C], ["ws-b", "ws-b", "ws-a"]);
    // ws-b appears once, ws-a once, ws-c appended
    assert.deepEqual(
      result.map((c) => c.id),
      ["ws-b", "ws-a", "ws-c"],
    );
  });
});
