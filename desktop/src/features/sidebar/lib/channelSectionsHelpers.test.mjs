import assert from "node:assert/strict";
import test from "node:test";

import {
  isChannelSectionsAllowlistReady,
  scopeChannelSectionsToKnownChannels,
  swapSectionOrder,
} from "./channelSectionsHelpers.ts";

function makeStore(sections, assignments = {}) {
  return { version: 1, sections, assignments };
}

test("isChannelSectionsAllowlistReady: explicit ready wins over empty set", () => {
  assert.equal(isChannelSectionsAllowlistReady(new Set(), true), true);
  assert.equal(isChannelSectionsAllowlistReady(new Set(["a"]), false), false);
  assert.equal(isChannelSectionsAllowlistReady(new Set(["a"])), true);
  assert.equal(isChannelSectionsAllowlistReady(new Set()), false);
});

test("scopeChannelSectionsToKnownChannels: strips foreign channel ids", () => {
  const store = makeStore([{ id: "s1", name: "One", order: 0 }], {
    "chan-a": "s1",
    "chan-b-foreign": "s1",
  });
  const filtered = scopeChannelSectionsToKnownChannels(
    store,
    new Set(["chan-a"]),
  );
  assert.deepEqual(filtered.assignments, { "chan-a": "s1" });
  assert.equal(filtered.sections.length, 1);
  assert.equal(filtered.sections[0].id, "s1");
});

test("scopeChannelSectionsToKnownChannels: drops sections that only held foreign channels", () => {
  const store = makeStore(
    [
      { id: "s-local", name: "Local", order: 0 },
      { id: "s-foreign", name: "From B", order: 1 },
      { id: "s-empty", name: "Empty", order: 2 },
    ],
    {
      "chan-a": "s-local",
      "chan-b-foreign": "s-foreign",
    },
  );
  const scoped = scopeChannelSectionsToKnownChannels(
    store,
    new Set(["chan-a"]),
  );
  assert.deepEqual(scoped.assignments, { "chan-a": "s-local" });
  assert.deepEqual(
    scoped.sections.map((s) => s.id),
    ["s-local", "s-empty"],
  );
});

test("scopeChannelSectionsToKnownChannels: empty allowlist is a no-op until channelsReady", () => {
  const store = makeStore([{ id: "s1", name: "One", order: 0 }], {
    "chan-a": "s1",
  });
  assert.equal(scopeChannelSectionsToKnownChannels(store, new Set()), store);
  assert.equal(scopeChannelSectionsToKnownChannels(store, null), store);
  assert.equal(scopeChannelSectionsToKnownChannels(store, undefined), store);
  assert.deepEqual(
    scopeChannelSectionsToKnownChannels(store, new Set(), true).assignments,
    {},
  );
});

test("scopeChannelSectionsToKnownChannels: returns same reference when nothing stripped", () => {
  const store = makeStore([{ id: "s1", name: "One", order: 0 }], {
    "chan-a": "s1",
  });
  assert.equal(
    scopeChannelSectionsToKnownChannels(store, new Set(["chan-a", "chan-b"])),
    store,
  );
});

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
