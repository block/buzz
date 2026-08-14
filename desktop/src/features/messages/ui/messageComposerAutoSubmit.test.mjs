/**
 * Unit tests for `scheduleSettleGatedAutoSubmit` — the auto-submit scheduler
 * that fires a ?autoSend draft submit exactly once, after link-preview settling
 * finishes.
 *
 * Imports and exercises the ACTUAL source helper. Regression guard for the
 * auto-send-drop blocker (PR #5245, Blocker A): a confirmed draft with a
 * supported link is normally still settling at mount, so an immediate submit
 * bails on the pending guard. The prior one-shot `setTimeout(0)` consumed the
 * trigger and silently dropped the draft. The scheduler must instead poll while
 * pending and submit exactly once when settling clears — never zero, never
 * twice.
 *
 * A controllable fake timer drives the poll deterministically, so there is no
 * real-time flakiness (the E2E form could not reliably send inside the ~350 ms
 * window headless).
 */

import assert from "node:assert/strict";
import test from "node:test";
import { scheduleSettleGatedAutoSubmit } from "./messageComposerAutoSubmit.ts";

// Minimal deterministic timer: records scheduled callbacks so the test can
// advance them one "tick" at a time and assert exact call counts.
function makeFakeTimers() {
  const pending = new Map();
  let nextId = 1;
  return {
    set(fn, _ms) {
      const id = nextId++;
      pending.set(id, fn);
      return id;
    },
    clear(id) {
      pending.delete(id);
    },
    // Fire the earliest-scheduled still-pending callback.
    tick() {
      const [id, fn] = pending.entries().next().value ?? [];
      if (id === undefined) return false;
      pending.delete(id);
      fn();
      return true;
    },
    pendingCount() {
      return pending.size;
    },
  };
}

test("submits once immediately when nothing is pending", () => {
  const timers = makeFakeTimers();
  let submits = 0;
  scheduleSettleGatedAutoSubmit({
    isPending: () => false,
    submit: () => submits++,
    timers,
  });
  timers.tick(); // fire the initial setTimeout(0)
  assert.equal(submits, 1);
  assert.equal(timers.pendingCount(), 0, "no retry should be scheduled");
});

test("waits while settling then submits exactly once (the drop-guard)", () => {
  const timers = makeFakeTimers();
  let submits = 0;
  let pending = true; // still settling at mount
  scheduleSettleGatedAutoSubmit({
    isPending: () => pending,
    submit: () => submits++,
    timers,
  });
  timers.tick(); // initial attempt: pending → reschedules, does NOT submit
  assert.equal(submits, 0, "must not send while a snapshot is still pending");
  assert.equal(timers.pendingCount(), 1, "a retry must be scheduled");

  timers.tick(); // retry: still pending
  assert.equal(submits, 0);

  pending = false; // settling finished
  timers.tick(); // retry: fires the send
  assert.equal(submits, 1, "must send exactly once after settling clears");
  assert.equal(timers.pendingCount(), 0);
});

test("cleanup before settling finishes cancels the submit (no orphan send)", () => {
  const timers = makeFakeTimers();
  let submits = 0;
  const cleanup = scheduleSettleGatedAutoSubmit({
    isPending: () => true,
    submit: () => submits++,
    timers,
  });
  timers.tick(); // initial attempt reschedules a retry
  assert.equal(timers.pendingCount(), 1);
  cleanup(); // unmount
  assert.equal(
    timers.pendingCount(),
    0,
    "cleanup must clear the pending retry",
  );
  assert.equal(submits, 0);
});
