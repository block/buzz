import assert from "node:assert/strict";
import test from "node:test";

import { fibreActivityAt, resolveFibreSort, sortFibres } from "./fibreSort.ts";

const older = {
  id: "old",
  score: 90,
  updatedAt: 100,
  artifacts: [{ createdAt: 100 }],
};
const newerLow = {
  id: "new",
  score: 20,
  updatedAt: 200,
  artifacts: [{ createdAt: 200 }],
};

test("only Important is ranked by default; the rest lead with newest", () => {
  assert.equal(resolveFibreSort("important", null), "priority");
  assert.equal(resolveFibreSort("hot", null), "newest");
  assert.equal(resolveFibreSort("other", null), "newest");
  assert.equal(resolveFibreSort("done", null), "newest");
});

test("an explicit preference wins on every tab", () => {
  assert.equal(resolveFibreSort("done", "priority"), "priority");
  assert.equal(resolveFibreSort("important", "newest"), "newest");
  assert.equal(resolveFibreSort("hot", "priority"), "priority");
});

test("priority sort is score desc then updatedAt", () => {
  const ordered = sortFibres([newerLow, older], "priority");
  assert.deepEqual(
    ordered.map((fibre) => fibre.id),
    ["old", "new"],
  );
});

test("newest sort uses latest artifact time", () => {
  const ordered = sortFibres([older, newerLow], "newest");
  assert.deepEqual(
    ordered.map((fibre) => fibre.id),
    ["new", "old"],
  );
});

test("done recency uses updatedAt not artifact time", () => {
  const completedEarlier = {
    id: "a",
    score: 10,
    updatedAt: 50,
    artifacts: [{ createdAt: 500 }],
  };
  const completedLater = {
    id: "b",
    score: 10,
    updatedAt: 80,
    artifacts: [{ createdAt: 10 }],
  };
  const ordered = sortFibres(
    [completedEarlier, completedLater],
    "newest",
    "updated",
  );
  assert.deepEqual(
    ordered.map((fibre) => fibre.id),
    ["b", "a"],
  );
});

test("fibreActivityAt prefers the latest artifact", () => {
  assert.equal(
    fibreActivityAt({
      updatedAt: 1,
      artifacts: [{ createdAt: 5 }, { createdAt: 9 }],
    }),
    9,
  );
});
