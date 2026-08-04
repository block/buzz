import assert from "node:assert/strict";
import test from "node:test";

import {
  CHANNELS_BLOCK_ID,
  appendSectionToStore,
  applySidebarBlockOrder,
  getSidebarBlockOrder,
  normalizeChannelsBlockIndex,
  removeSectionFromStore,
  resolveChannelsBlockIndex,
  swapBlockOrder,
  swapSectionOrder,
} from "./channelSectionsHelpers.ts";

function makeStore(sections, assignments = {}, channelsBlockIndex) {
  return {
    version: 1,
    sections,
    assignments,
    ...(channelsBlockIndex !== undefined ? { channelsBlockIndex } : {}),
  };
}

function makeSection(id, name, order) {
  return { id, name, order };
}

test("resolveChannelsBlockIndex: missing maps to after all categories", () => {
  const store = makeStore([makeSection("a", "A", 0), makeSection("b", "B", 1)]);
  assert.equal(resolveChannelsBlockIndex(store), 2);
});

test("resolveChannelsBlockIndex: clamps out of range", () => {
  const store = makeStore([makeSection("a", "A", 0)], {}, 99);
  assert.equal(resolveChannelsBlockIndex(store), 1);
  assert.equal(
    resolveChannelsBlockIndex(makeStore([makeSection("a", "A", 0)], {}, -3)),
    0,
  );
});

test("resolveChannelsBlockIndex: non-integers map to legacy layout", () => {
  const store = makeStore([makeSection("a", "A", 0), makeSection("b", "B", 1)]);
  assert.equal(
    resolveChannelsBlockIndex({ ...store, channelsBlockIndex: 1.5 }),
    2,
  );
});

test("normalizeChannelsBlockIndex: malformed and out of range → undefined", () => {
  assert.equal(normalizeChannelsBlockIndex(undefined, 2), undefined);
  assert.equal(normalizeChannelsBlockIndex("1", 2), undefined);
  assert.equal(normalizeChannelsBlockIndex(Number.NaN, 2), undefined);
  assert.equal(normalizeChannelsBlockIndex(-1, 2), undefined);
  assert.equal(normalizeChannelsBlockIndex(3, 2), undefined);
  // Non-integers are malformed — never truncate into a different position.
  assert.equal(normalizeChannelsBlockIndex(1.5, 2), undefined);
  assert.equal(normalizeChannelsBlockIndex(1.9, 2), undefined);
  assert.equal(normalizeChannelsBlockIndex(0, 2), 0);
  assert.equal(normalizeChannelsBlockIndex(1, 2), 1);
  assert.equal(normalizeChannelsBlockIndex(2, 2), 2);
});

test("getSidebarBlockOrder: default is categories then Channels", () => {
  const store = makeStore([makeSection("a", "A", 0), makeSection("b", "B", 1)]);
  assert.deepEqual(getSidebarBlockOrder(store), ["a", "b", CHANNELS_BLOCK_ID]);
});

test("getSidebarBlockOrder: respects channelsBlockIndex mid-lane", () => {
  const store = makeStore(
    [makeSection("a", "A", 0), makeSection("b", "B", 1)],
    {},
    1,
  );
  assert.deepEqual(getSidebarBlockOrder(store), ["a", CHANNELS_BLOCK_ID, "b"]);
});

test("applySidebarBlockOrder: reorders sections and records index", () => {
  const prev = makeStore([makeSection("a", "A", 0), makeSection("b", "B", 1)]);
  const next = applySidebarBlockOrder(prev, ["b", CHANNELS_BLOCK_ID, "a"]);
  assert.equal(next.channelsBlockIndex, 1);
  const byId = Object.fromEntries(next.sections.map((s) => [s.id, s.order]));
  assert.equal(byId.b, 0);
  assert.equal(byId.a, 1);
  assert.deepEqual(getSidebarBlockOrder(next), ["b", CHANNELS_BLOCK_ID, "a"]);
});

test("swapBlockOrder: moves category across Channels boundary", () => {
  const store = makeStore(
    [makeSection("a", "A", 0), makeSection("b", "B", 1)],
    {},
    1,
  );
  // [A, Channels, B] — move B up across Channels → [A, B, Channels]
  const up = swapBlockOrder(store, "b", "up");
  assert.notEqual(up, null);
  assert.deepEqual(getSidebarBlockOrder(up), ["a", "b", CHANNELS_BLOCK_ID]);

  // [A, Channels, B] — move Channels up → [Channels, A, B]
  const chUp = swapBlockOrder(store, CHANNELS_BLOCK_ID, "up");
  assert.notEqual(chUp, null);
  assert.deepEqual(getSidebarBlockOrder(chUp), [CHANNELS_BLOCK_ID, "a", "b"]);
});

test("swapBlockOrder: boundary no-ops return null", () => {
  const store = makeStore([makeSection("a", "A", 0)], {}, 0);
  // [Channels, A]
  assert.equal(swapBlockOrder(store, CHANNELS_BLOCK_ID, "up"), null);
  assert.equal(swapBlockOrder(store, "a", "down"), null);
});

test("removeSectionFromStore: deleting category before Channels keeps relative place", () => {
  // [A, Channels, B] channelsBlockIndex=1 — delete A → [Channels, B]
  const store = makeStore(
    [makeSection("a", "A", 0), makeSection("b", "B", 1)],
    { ch1: "a", ch2: "b" },
    1,
  );
  const next = removeSectionFromStore(store, "a");
  assert.deepEqual(
    next.sections.map((s) => s.id),
    ["b"],
  );
  assert.equal(next.channelsBlockIndex, 0);
  assert.deepEqual(getSidebarBlockOrder(next), [CHANNELS_BLOCK_ID, "b"]);
  assert.equal(next.assignments.ch1, undefined);
  assert.equal(next.assignments.ch2, "b");
});

test("removeSectionFromStore: deleting category after Channels keeps index", () => {
  // [A, Channels, B] — delete B → [A, Channels]
  const store = makeStore(
    [makeSection("a", "A", 0), makeSection("b", "B", 1)],
    {},
    1,
  );
  const next = removeSectionFromStore(store, "b");
  assert.deepEqual(getSidebarBlockOrder(next), ["a", CHANNELS_BLOCK_ID]);
  assert.equal(next.channelsBlockIndex, 1);
});

test("appendSectionToStore: does not shift Channels among existing categories", () => {
  // [A, Channels, B] — create C → [A, Channels, B, C]
  const store = makeStore(
    [makeSection("a", "A", 0), makeSection("b", "B", 1)],
    {},
    1,
  );
  const next = appendSectionToStore(store, {
    id: "c",
    name: "C",
    order: 0,
  });
  assert.equal(next.channelsBlockIndex, 1);
  assert.deepEqual(getSidebarBlockOrder(next), [
    "a",
    CHANNELS_BLOCK_ID,
    "b",
    "c",
  ]);
});

test("appendSectionToStore: default layout keeps Channels last", () => {
  const store = makeStore([makeSection("a", "A", 0)]);
  const next = appendSectionToStore(store, {
    id: "b",
    name: "B",
    order: 0,
  });
  assert.equal(next.channelsBlockIndex, undefined);
  assert.deepEqual(getSidebarBlockOrder(next), ["a", "b", CHANNELS_BLOCK_ID]);
});

test("move up succeeds: middle section swaps order with the one above", () => {
  const store = makeStore([
    makeSection("a", "A", 0),
    makeSection("b", "B", 1),
    makeSection("c", "C", 2),
  ]);
  const result = swapSectionOrder(store, "b", "up");
  assert.notEqual(result, null);
  const byId = Object.fromEntries(result.sections.map((s) => [s.id, s.order]));
  assert.equal(byId.b, 0);
  assert.equal(byId.a, 1);
  assert.equal(byId.c, 2);
});

test("move down succeeds: middle section swaps order with the one below", () => {
  const store = makeStore([
    makeSection("a", "A", 0),
    makeSection("b", "B", 1),
    makeSection("c", "C", 2),
  ]);
  const result = swapSectionOrder(store, "b", "down");
  assert.notEqual(result, null);
  const byId = Object.fromEntries(result.sections.map((s) => [s.id, s.order]));
  assert.equal(byId.b, 2);
  assert.equal(byId.c, 1);
  assert.equal(byId.a, 0);
});

test("move up at top boundary returns null", () => {
  const store = makeStore([makeSection("a", "A", 0), makeSection("b", "B", 1)]);
  assert.equal(swapSectionOrder(store, "a", "up"), null);
});

test("move down at last category swaps across Channels (not a no-op)", () => {
  // Unified lane is [A, B, Channels]; moving B down crosses Channels.
  const store = makeStore([makeSection("a", "A", 0), makeSection("b", "B", 1)]);
  const result = swapSectionOrder(store, "b", "down");
  assert.notEqual(result, null);
  assert.deepEqual(getSidebarBlockOrder(result), ["a", CHANNELS_BLOCK_ID, "b"]);
});

test("non-existent section returns null", () => {
  const store = makeStore([makeSection("a", "A", 0)]);
  assert.equal(swapSectionOrder(store, "z", "up"), null);
});

test("single section move up returns null", () => {
  const store = makeStore([makeSection("a", "A", 0)]);
  assert.equal(swapSectionOrder(store, "a", "up"), null);
});

test("single section move down swaps with Channels", () => {
  // Unified lane is [A, Channels]; moving A down is valid.
  const store = makeStore([makeSection("a", "A", 0)]);
  const result = swapSectionOrder(store, "a", "down");
  assert.notEqual(result, null);
  assert.deepEqual(getSidebarBlockOrder(result), [CHANNELS_BLOCK_ID, "a"]);
});
