import assert from "node:assert/strict";
import test from "node:test";

import { applyReactionState, isDuplicateReactionError } from "./noteActions.ts";

const NOTE_ID = "n".repeat(64);

test("applyReactionState adds a current-user reaction", () => {
  const next = applyReactionState(undefined, NOTE_ID, true);
  assert.deepEqual(next.get(NOTE_ID), {
    count: 1,
    reactedByCurrentUser: true,
  });
});

test("applyReactionState removes a current-user reaction", () => {
  const current = new Map([
    [NOTE_ID, { count: 2, reactedByCurrentUser: true }],
  ]);
  const next = applyReactionState(current, NOTE_ID, false);
  assert.deepEqual(next.get(NOTE_ID), {
    count: 1,
    reactedByCurrentUser: false,
  });
});

test("applyReactionState keeps count stable for no-op transitions", () => {
  const current = new Map([
    [NOTE_ID, { count: 2, reactedByCurrentUser: false }],
  ]);
  const next = applyReactionState(current, NOTE_ID, false);
  assert.deepEqual(next.get(NOTE_ID), {
    count: 2,
    reactedByCurrentUser: false,
  });
});

test("applyReactionState never decrements below zero", () => {
  const next = applyReactionState(undefined, NOTE_ID, false);
  assert.deepEqual(next.get(NOTE_ID), {
    count: 0,
    reactedByCurrentUser: false,
  });
});

test("isDuplicateReactionError detects relay duplicate responses", () => {
  assert.equal(
    isDuplicateReactionError(
      new Error("relay rejected event: duplicate: reaction already exists"),
    ),
    true,
  );
});

test("isDuplicateReactionError rejects unrelated errors and non-errors", () => {
  assert.equal(isDuplicateReactionError(new Error("network failed")), false);
  assert.equal(
    isDuplicateReactionError("duplicate: reaction already exists"),
    false,
  );
});
