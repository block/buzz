import assert from "node:assert/strict";
import test from "node:test";
import { startScreenPublisher } from "./screenPublisher.ts";

const flush = () => new Promise((resolve) => setImmediate(resolve));
function harness(overrides = {}) {
  const events = [];
  const timers = [];
  const errors = [];
  const stop = startScreenPublisher({
    capture: async () => ({ id: "frame" }),
    publish: async (event, current) => {
      if (current()) events.push(event);
    },
    fail: async (error) => {
      errors.push(error);
    },
    schedule: (callback, delay) => {
      timers.push({ callback, delay });
      return callback;
    },
    unschedule: (timer) => {
      const index = timers.findIndex((entry) => entry.callback === timer);
      if (index >= 0) timers.splice(index, 1);
    },
    ...overrides,
  });
  return { events, timers, errors, stop };
}

test("only the next frame is scheduled after publication settles", async () => {
  let resolvePublish;
  const h = harness({
    publish: () =>
      new Promise((resolve) => {
        resolvePublish = resolve;
      }),
  });
  await flush();
  assert.equal(h.timers.length, 0);
  resolvePublish();
  await flush();
  assert.equal(h.timers.length, 1);
  assert.equal(h.timers[0].delay, 0);
  h.stop();
  assert.equal(h.timers.length, 0);
});

test("a capture completing after stop is never published", async () => {
  let resolveCapture;
  const h = harness({
    capture: () =>
      new Promise((resolve) => {
        resolveCapture = resolve;
      }),
  });
  h.stop();
  resolveCapture({ id: "late" });
  await flush();
  assert.deepEqual(h.events, []);
  assert.deepEqual(h.timers, []);
});

test("failure is terminal and reaches the native stop/error path", async () => {
  const failure = new Error("permission denied");
  const h = harness({
    capture: async () => {
      throw failure;
    },
  });
  await flush();
  assert.deepEqual(h.errors, [failure]);
  assert.deepEqual(h.timers, []);
  h.stop();
});

function video(id, capturedAt, startedAt = 10_000) {
  return {
    id,
    content: JSON.stringify({
      type: "video",
      share: { started_at: startedAt },
      frames: [{ timestamp: (capturedAt - startedAt) * 1000 }],
    }),
  };
}

async function next(h, advance = () => {}) {
  const timer = h.timers.shift();
  assert.ok(timer, "publisher must have scheduled its next step");
  advance(timer.delay);
  timer.callback();
  await flush();
}

test("a zero-second quota refusal backs off and discards old captured pixels", async () => {
  let now = 10_000;
  const captures = [
    video("refused", now),
    video("queued-before-cooldown", now + 100),
    video("fresh", now + 300),
  ];
  const published = [];
  const h = harness({
    now: () => now,
    capture: async () => captures.shift() ?? null,
    publish: async (event) => {
      published.push(event.id);
      if (event.id === "refused")
        throw new Error("rate-limited: quota exceeded; retry in 0s");
    },
  });
  await flush();
  assert.equal(h.timers[0].delay, 250);
  assert.deepEqual(h.errors, []);
  await next(h, (delay) => {
    now += delay;
  });
  assert.deepEqual(published, ["refused"]);
  await next(h, () => {
    now += 50;
  });
  assert.deepEqual(published, ["refused", "fresh"]);
  assert.deepEqual(h.errors, []);
  h.stop();
});

test("persistent short refusals stop after bounded retries", async () => {
  let now = 10_000;
  const failure = new Error("rate-limited: quota exceeded; retry in 0s");
  let attempts = 0;
  const h = harness({
    now: () => now,
    capture: async () => video(`frame-${attempts}`, now),
    publish: async () => {
      attempts++;
      throw failure;
    },
  });
  await flush();
  for (let index = 0; index < 6; index++)
    await next(h, (delay) => {
      now += delay;
    });
  assert.equal(attempts, 7);
  assert.deepEqual(h.errors, [failure]);
  assert.deepEqual(h.timers, []);
  h.stop();
});

test("cooldown time is bounded even when each refusal has a five-second hint", async () => {
  let now = 10_000;
  const failure = new Error("rate-limited: quota exceeded; retry in 5s");
  let attempts = 0;
  const h = harness({
    now: () => now,
    capture: async () => video("fresh", now),
    publish: async () => {
      attempts++;
      throw failure;
    },
  });
  await flush();
  await next(h, (delay) => {
    now += delay;
  });
  await next(h, (delay) => {
    now += delay;
  });
  assert.equal(attempts, 3);
  assert.equal(now, 20_500);
  assert.deepEqual(h.errors, [failure]);
  assert.deepEqual(h.timers, []);
  h.stop();
});

test("stopping during cooldown never resumes sharing", async () => {
  let captures = 0;
  const h = harness({
    capture: async () => {
      captures++;
      return video("frame", 10_000);
    },
    publish: async () => {
      throw new Error("rate-limited: quota exceeded; retry in 1s");
    },
  });
  await flush();
  const late = h.timers[0].callback;
  h.stop();
  late();
  await flush();
  assert.deepEqual(h.events, []);
  assert.deepEqual(h.errors, []);
  assert.deepEqual(h.timers, []);
  assert.equal(captures, 1);
});

test("long or unrelated rejections remain terminal", async () => {
  for (const message of [
    "rate-limited: quota exceeded; retry in 60s",
    "rate-limited: shared admission unavailable",
    "rate-limited: quota exceeded; retry in 0s trailing text",
    "Screen sharing timed out",
    "WebSocket disconnected",
  ]) {
    const failure = new Error(message);
    const h = harness({
      publish: async () => {
        throw failure;
      },
    });
    await flush();
    assert.deepEqual(h.errors, [failure]);
    assert.deepEqual(h.timers, []);
    h.stop();
  }
});
