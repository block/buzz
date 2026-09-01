/**
 * What a burst of microphone taps leaves behind.
 *
 * Two properties, and both are about a microphone rather than about tidy
 * state: the device ends where the member last asked it to, and nothing ever
 * reports "muted" on behalf of a request the member has already superseded.
 */

import assert from "node:assert/strict";
import test from "node:test";

import { createMicRequestQueue } from "./micRequestQueue.ts";

/**
 * Let queued work reach `apply`.
 *
 * Requests are chained onto a promise, so the provider call is made in a later
 * microtask and never synchronously with the tap that asked for it.
 */
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

/** A controllable `apply`: records calls, settles when told. */
function deferredApply() {
  const calls = [];
  const pending = [];
  const apply = (enabled) =>
    new Promise((resolve, reject) => {
      calls.push(enabled);
      pending.push({ resolve, reject });
    });
  return { apply, calls, pending };
}

test("taps landing together cost one call, carrying the settled value", {
  timeout: 5_000,
}, async () => {
  const { apply, calls, pending } = deferredApply();
  const queue = createMicRequestQueue(apply);

  // Three taps in one tick. Each supersedes the last before any of them has
  // had a turn, so the first two are dropped without ever being applied.
  queue.request(true, () => {});
  queue.request(false, () => {});
  queue.request(true, () => {});
  await tick();

  assert.deepEqual(calls, [true], "one provider call, not three");

  // Settling it and letting the chain drain, rather than awaiting the chain
  // itself: a queue that applied every superseded request would leave calls
  // nothing ever settles, and awaiting it would hang here instead of failing.
  pending[0].resolve();
  await tick();

  assert.deepEqual(
    calls,
    [true],
    "the values passed through never reach the device",
  );
});

test("the device ends where the member last asked, not where the slowest call did", {
  timeout: 5_000,
}, async () => {
  const { apply, calls, pending } = deferredApply();
  const queue = createMicRequestQueue(apply);

  queue.request(true, () => {});
  const off = queue.request(false, () => {});
  await tick();
  pending[0].resolve();
  await off;

  assert.equal(
    calls.at(-1),
    false,
    "the last applied value is the last one requested",
  );
});

test("a superseded request's failure does not report muted", {
  timeout: 5_000,
}, async () => {
  const { apply, pending } = deferredApply();
  const queue = createMicRequestQueue(apply);

  let reportedMuted = false;
  const mark = () => {
    reportedMuted = true;
  };
  // The member unmutes; that call reaches the device and is still running when
  // they tap again. The first attempt then fails, after it stopped speaking
  // for them.
  queue.request(true, mark);
  await tick();
  const newer = queue.request(true, mark);
  pending[0].reject(new Error("device busy"));
  await tick();
  pending[1].resolve();
  await newer;

  assert.equal(
    reportedMuted,
    false,
    "a stale rejection must not label a newer request's microphone as muted",
  );
});

test("the newest request's failure does report muted", {
  timeout: 5_000,
}, async () => {
  // The refusal above means nothing without this: a fence that suppressed
  // every failure would satisfy it while leaving the indicator lying the
  // other way, claiming an open microphone that never opened.
  const { apply, pending } = deferredApply();
  const queue = createMicRequestQueue(apply);

  let reportedMuted = false;
  const only = queue.request(true, () => {
    reportedMuted = true;
  });
  await tick();
  pending[0].reject(new Error("permission denied"));
  await only;

  assert.equal(reportedMuted, true);
});

test("superseding drops a queued request before it reaches the device", {
  timeout: 5_000,
}, async () => {
  const { apply, calls, pending } = deferredApply();
  const queue = createMicRequestQueue(apply);

  queue.request(true, () => {});
  await tick();
  // Queued behind the in-flight call, then the room changes underneath it.
  const queued = queue.request(false, () => {});
  queue.supersede();
  pending[0].resolve();
  await queued;

  assert.deepEqual(
    calls,
    [true],
    "a request superseded by a room change never reaches the device",
  );
});

test("a failure does not wedge the queue", { timeout: 5_000 }, async () => {
  // The queue is one promise chain, so a rejection escaping it would silently
  // stop every later request — the microphone button would go dead after the
  // first denied permission prompt.
  const { apply, calls, pending } = deferredApply();
  const queue = createMicRequestQueue(apply);

  const failing = queue.request(true, () => {});
  await tick();
  pending[0].reject(new Error("denied"));
  await failing;

  const next = queue.request(true, () => {});
  await tick();
  pending[1].resolve();
  await next;

  assert.deepEqual(calls, [true, true], "a later request still reaches apply");
});
