/**
 * Unit tests for the save coalescer helper.
 *
 * Each test controls the in-flight save duration with a deferred promise so
 * it can precisely verify interleaving: one edit during flight, multiple
 * overwrites, cancellation, etc.
 */
import assert from "node:assert/strict";
import test from "node:test";

import { createSaveCoalescer } from "./saveCoalescer.ts";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

// Drain one microtask queue turn so the async drain() loop can advance.
const tick = () => new Promise((r) => setTimeout(r, 0));

test("saveCoalescer_single_edit_is_persisted_and_saved_is_applied", async () => {
  const persisted = [];
  const saved = [];

  const coalescer = createSaveCoalescer(
    async (v) => {
      persisted.push(v);
      return { ...v, fromServer: true };
    },
    () => {},
    (v) => saved.push(v),
  );

  coalescer.enqueue({ model: "claude" });
  await tick();
  await tick();

  assert.equal(persisted.length, 1);
  assert.equal(persisted[0].model, "claude");
  assert.equal(saved.length, 1);
  assert.equal(saved[0].fromServer, true);
});

test("saveCoalescer_rapid_edits_coalesce_second_is_drained_after_first", async () => {
  const d = deferred();
  const persisted = [];

  const coalescer = createSaveCoalescer(
    async (v) => {
      if (v.n === 0) await d.promise;
      persisted.push(v.n);
      return v;
    },
    () => {},
    () => {},
  );

  coalescer.enqueue({ n: 0 });
  // Second edit arrives while first save is still in flight.
  coalescer.enqueue({ n: 1 });

  d.resolve();
  await tick();
  await tick();

  assert.deepEqual(persisted, [0, 1]);
});

test("saveCoalescer_three_rapid_edits_only_first_and_last_are_persisted", async () => {
  const d = deferred();
  const persisted = [];

  const coalescer = createSaveCoalescer(
    async (v) => {
      if (v.n === 0) await d.promise;
      persisted.push(v.n);
      return v;
    },
    () => {},
    () => {},
  );

  coalescer.enqueue({ n: 0 });
  coalescer.enqueue({ n: 1 }); // overwritten before drain picks it up
  coalescer.enqueue({ n: 2 }); // overwrites n:1

  d.resolve();
  await tick();
  await tick();

  // n:1 was never the pending value when the drain loop checked; only n:2.
  assert.deepEqual(persisted, [0, 2]);
});

test("saveCoalescer_onSaved_suppressed_for_first_when_second_is_pending", async () => {
  const d = deferred();
  const savedCalls = [];

  const coalescer = createSaveCoalescer(
    async (v) => {
      if (v.n === 0) await d.promise;
      return v;
    },
    () => {},
    (v) => savedCalls.push(v.n),
  );

  coalescer.enqueue({ n: 0 });
  // Queue n:1 before n:0 resolves — onSaved for n:0 should be suppressed.
  coalescer.enqueue({ n: 1 });

  d.resolve();
  await tick();
  await tick();

  // Only n:1 (the final save round) triggers onSaved.
  assert.deepEqual(savedCalls, [1]);
});

test("saveCoalescer_isSaving_transitions_true_then_false", async () => {
  const d = deferred();
  const states = [];

  const coalescer = createSaveCoalescer(
    async (v) => {
      await d.promise;
      return v;
    },
    (s) => states.push(s),
    () => {},
  );

  coalescer.enqueue({ n: 0 });
  assert.deepEqual(states, [true]);

  d.resolve();
  await tick();
  await tick();

  assert.deepEqual(states, [true, false]);
});

test("saveCoalescer_cancel_prevents_onSaved_and_onSaving_false", async () => {
  const d = deferred();
  const states = [];
  const savedCalls = [];

  const coalescer = createSaveCoalescer(
    async (v) => {
      await d.promise;
      return v;
    },
    (s) => states.push(s),
    (v) => savedCalls.push(v),
  );

  coalescer.enqueue({ n: 0 });
  coalescer.cancel();
  d.resolve();
  await tick();
  await tick();

  // After cancel, neither onSaved nor onSaving(false) fire.
  assert.equal(savedCalls.length, 0);
  assert.equal(states.includes(false), false);
});

test("saveCoalescer_save_error_does_not_call_onSaved_but_drains_pending", async () => {
  const d = deferred();
  const persisted = [];
  const savedCalls = [];

  const coalescer = createSaveCoalescer(
    async (v) => {
      if (v.n === 0) {
        await d.promise;
        throw new Error("network error");
      }
      persisted.push(v.n);
      return v;
    },
    () => {},
    (v) => savedCalls.push(v.n),
  );

  coalescer.enqueue({ n: 0 });
  coalescer.enqueue({ n: 1 }); // pending while n:0 fails

  d.resolve();
  await tick();
  await tick();

  // n:0 errored — no onSaved for it; n:1 was drained and saved.
  assert.deepEqual(persisted, [1]);
  assert.deepEqual(savedCalls, [1]);
});
