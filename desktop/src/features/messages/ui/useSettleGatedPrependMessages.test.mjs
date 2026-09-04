import assert from "node:assert/strict";
import test from "node:test";

import { selectSettleGatedMessages } from "./useSettleGatedPrependMessages.ts";

const rows = (...ids) => ids.map((id) => ({ id }));

test("holds a pure older-history prepend", () => {
  const decision = selectSettleGatedMessages({
    admitted: rows("a", "b", "c"),
    next: rows("older-1", "older-2", "a", "b", "c"),
  });
  assert.equal(decision.kind, "hold");
  assert.deepEqual(
    decision.held.map(({ id }) => id),
    ["a", "b", "c"],
  );
});

test("held snapshot uses refreshed row objects for the admitted ids", () => {
  const editedA = { id: "a", edited: true };
  const decision = selectSettleGatedMessages({
    admitted: rows("a", "b"),
    next: [{ id: "older-1" }, editedA, { id: "b" }],
  });
  assert.equal(decision.kind, "hold");
  assert.equal(decision.held[0], editedA);
});

test("passes appends through immediately", () => {
  assert.equal(
    selectSettleGatedMessages({
      admitted: rows("a", "b"),
      next: rows("a", "b", "c"),
    }).kind,
    "pass",
  );
});

test("holds a simultaneous prepend+append until explicit release", () => {
  assert.equal(
    selectSettleGatedMessages({
      admitted: rows("a", "b"),
      next: rows("older-1", "a", "b", "sent"),
    }).kind,
    "hold",
  );
});

test("holds a prepend with deletions inside the admitted window", () => {
  assert.equal(
    selectSettleGatedMessages({
      admitted: rows("a", "b", "c"),
      next: rows("older-1", "a", "c"),
    }).kind,
    "hold",
  );
});

test("passes authoritative replacements through", () => {
  assert.equal(
    selectSettleGatedMessages({
      admitted: rows("a", "b"),
      next: rows("x", "y"),
    }).kind,
    "pass",
  );
});

test("passes identical snapshots through", () => {
  assert.equal(
    selectSettleGatedMessages({
      admitted: rows("a", "b"),
      next: rows("a", "b"),
    }).kind,
    "pass",
  );
});

test("passes when the previous snapshot was empty (initial load)", () => {
  assert.equal(
    selectSettleGatedMessages({
      admitted: [],
      next: rows("a", "b"),
    }).kind,
    "pass",
  );
});
