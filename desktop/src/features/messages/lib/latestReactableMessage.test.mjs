import assert from "node:assert/strict";
import test from "node:test";

import { findLatestReactableMessage } from "./latestReactableMessage.ts";

const KIND_SYSTEM_MESSAGE = 40099;
const KIND_MESSAGE = 9;

function entry(id, overrides = {}) {
  return { message: { id, kind: KIND_MESSAGE, pending: false, ...overrides } };
}

test("targets the newest message", () => {
  const entries = [entry("a"), entry("b"), entry("c")];
  assert.equal(findLatestReactableMessage(entries)?.id, "c");
});

test("skips pending sends and system rows from the end", () => {
  const entries = [
    entry("a"),
    entry("b"),
    entry("joined", { kind: KIND_SYSTEM_MESSAGE }),
    entry("pending", { pending: true }),
  ];
  assert.equal(findLatestReactableMessage(entries)?.id, "b");
});

test("returns null when nothing is reactable", () => {
  assert.equal(findLatestReactableMessage([]), null);
  assert.equal(
    findLatestReactableMessage([entry("pending", { pending: true })]),
    null,
  );
});
