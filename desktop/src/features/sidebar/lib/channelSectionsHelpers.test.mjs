import assert from "node:assert/strict";
import test from "node:test";

import { swapSectionOrder } from "./channelSectionsHelpers.ts";

function makeStore(sections, assignments = {}) {
  return { version: 1, sections, assignments };
}

function makeSection(id, name, order) {
  return { id, name, order };
}

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

test("move down at bottom boundary returns null", () => {
  const store = makeStore([makeSection("a", "A", 0), makeSection("b", "B", 1)]);
  assert.equal(swapSectionOrder(store, "b", "down"), null);
});

test("non-existent section returns null", () => {
  const store = makeStore([makeSection("a", "A", 0)]);
  assert.equal(swapSectionOrder(store, "z", "up"), null);
});

test("single section move up returns null", () => {
  const store = makeStore([makeSection("a", "A", 0)]);
  assert.equal(swapSectionOrder(store, "a", "up"), null);
});

test("single section move down returns null", () => {
  const store = makeStore([makeSection("a", "A", 0)]);
  assert.equal(swapSectionOrder(store, "a", "down"), null);
});

test("non-contiguous orders: swap uses actual order values not indices", () => {
  const store = makeStore([
    makeSection("a", "A", 0),
    makeSection("b", "B", 5),
    makeSection("c", "C", 10),
  ]);
  const result = swapSectionOrder(store, "b", "up");
  assert.notEqual(result, null);
  const byId = Object.fromEntries(result.sections.map((s) => [s.id, s.order]));
  assert.equal(byId.b, 0);
  assert.equal(byId.a, 5);
  assert.equal(byId.c, 10);
});
