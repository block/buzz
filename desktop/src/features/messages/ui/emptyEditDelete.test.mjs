import assert from "node:assert/strict";
import test from "node:test";

import { resolveEmptyEditDelete } from "./emptyEditDelete.ts";

test("deletes the edited message when a handler is wired", () => {
  assert.equal(resolveEmptyEditDelete("evt-123", true), "evt-123");
});

test("no-op when no delete handler is wired (never destroys silently)", () => {
  assert.equal(resolveEmptyEditDelete("evt-123", false), null);
});

test("no-op when there is no message loaded for editing", () => {
  assert.equal(resolveEmptyEditDelete(null, true), null);
  assert.equal(resolveEmptyEditDelete(undefined, true), null);
});

test("no-op on a blank target id", () => {
  assert.equal(resolveEmptyEditDelete("", true), null);
});

test("requires both a target and a handler", () => {
  assert.equal(resolveEmptyEditDelete(null, false), null);
  assert.equal(resolveEmptyEditDelete("", false), null);
});
