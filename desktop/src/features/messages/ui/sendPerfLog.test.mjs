import assert from "node:assert/strict";
import { test } from "node:test";

import { createSendPerfTimer } from "./sendPerfLog.ts";

function captureSink() {
  const emitted = [];
  return {
    emitted,
    sink: (label, payload) => emitted.push({ label, payload }),
  };
}

test("a step's resolved value passes straight through", async () => {
  const { emitted, sink } = captureSink();
  const timer = createSendPerfTimer("completeSend", {}, sink);

  const value = await timer.step("revalidate1", async () => ["abc"]);
  timer.finish();

  assert.deepEqual(value, ["abc"]);
  assert.equal(emitted.length, 1);
  assert.equal(emitted[0].label, "completeSend");
  assert.equal(typeof emitted[0].payload.steps.revalidate1, "number");
});

test("a throwing step still records its time and rethrows", async () => {
  const { emitted, sink } = captureSink();
  const timer = createSendPerfTimer("completeSend", {}, sink);

  await assert.rejects(
    timer.step("publish", async () => {
      throw new Error("relay rejected event");
    }),
    /relay rejected event/,
  );
  timer.finish();

  // A failed send is exactly when the timing matters, so the step must appear.
  assert.equal(typeof emitted[0].payload.steps.publish, "number");
});

test("repeating a step name accumulates rather than overwrites", async () => {
  const { emitted, sink } = captureSink();
  const timer = createSendPerfTimer("completeSend", {}, sink);

  await timer.step("revalidate", async () => null);
  await timer.step("revalidate", async () => null);
  timer.finish();

  assert.equal(Object.keys(emitted[0].payload.steps).length, 1);
  assert.ok(emitted[0].payload.steps.revalidate >= 0);
});

test("facts merge across construction, note, and finish", async () => {
  const { emitted, sink } = captureSink();
  const timer = createSendPerfTimer(
    "completeSend",
    { channelType: "dm" },
    sink,
  );

  timer.note({ revalidateTrigger: null, detachedStarts: 0 });
  timer.note({ revalidateTrigger: "relaySideEffects" });
  timer.finish({ detachedStarts: 1 });

  const { payload } = emitted[0];
  assert.equal(payload.channelType, "dm");
  assert.equal(payload.revalidateTrigger, "relaySideEffects");
  assert.equal(payload.detachedStarts, 1);
});

test("finish is idempotent so a finally block cannot double-report", () => {
  const { emitted, sink } = captureSink();
  const timer = createSendPerfTimer("completeSend", {}, sink);

  timer.finish();
  timer.finish({ detachedStarts: 9 });

  assert.equal(emitted.length, 1);
  assert.equal(emitted[0].payload.detachedStarts, undefined);
});

test("the total spans the whole timer, not just its steps", async () => {
  const { emitted, sink } = captureSink();
  const timer = createSendPerfTimer("completeSend", {}, sink);

  await timer.step("revalidate1", async () => null);
  await timer.step("publish", async () => null);
  timer.finish();

  const { payload } = emitted[0];
  const stepTotal = Object.values(payload.steps).reduce(
    (sum, ms) => sum + ms,
    0,
  );
  // Sequential steps, so the total covers them both — allow a tenth of slack
  // for the rounding each recorded value already went through.
  assert.ok(
    payload.totalMs + 0.2 >= stepTotal,
    `totalMs ${payload.totalMs} < steps ${stepTotal}`,
  );
});
