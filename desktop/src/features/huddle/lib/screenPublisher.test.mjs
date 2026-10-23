import assert from "node:assert/strict";
import test from "node:test";
import { startScreenPublisher } from "./screenPublisher.ts";
import { PublishCanceledError } from "@/shared/api/relayEventPublisher";

const flush = () => new Promise((resolve) => setImmediate(resolve));
function harness(overrides = {}) {
  const events = [];
  const timers = [];
  const errors = [];
  const stop = startScreenPublisher({
    capture: async () => video("frame", 10_000),
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
    now: () => 10_000,
    ...overrides,
  });
  return { events, timers, errors, stop };
}

test("a bounded publication pipeline keeps capturing while acknowledgements are pending", async () => {
  const pending = [];
  const h = harness({
    publish: () =>
      new Promise((resolve) => {
        pending.push(resolve);
      }),
  });
  await flush();
  assert.equal(h.timers.length, 1);
  for (let index = 1; index < 4; index++) await next(h);
  assert.equal(pending.length, 4, "only four publications may be in flight");
  assert.equal(h.timers.length, 0);
  pending.shift()();
  await flush();
  assert.equal(h.timers.length, 1);
  assert.equal(h.timers[0].delay, 0);
  h.stop();
  for (const resolve of pending) resolve();
  await flush();
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

test("quota retirement invalidates every waiting send and requests fresh native dependencies", async () => {
  let now = 10_000;
  const requests = [];
  const pending = [];
  let captureId = 0;
  const h = harness({
    now: () => now,
    capture: async (refresh) => {
      requests.push(refresh);
      return video(`frame-${captureId++}`, now);
    },
    publish: (_event, current) =>
      new Promise((resolve, reject) =>
        pending.push({ resolve, reject, current }),
      ),
  });
  await flush();
  await next(h);
  assert.equal(pending.length, 2);
  pending[0].reject(new Error("rate-limited: quota exceeded; retry in 0s"));
  await flush();
  assert.equal(
    pending[1].current(),
    false,
    "queued pre-cooldown pixels must not send",
  );
  pending[1].resolve();
  await flush();
  await next(h, (delay) => {
    now += delay;
  });
  assert.equal(requests.at(-1), true);
  assert.equal(h.errors.length, 0);
  h.stop();
  for (const item of pending) item.resolve();
  await flush();
  assert.equal(h.timers.length, 0);
});

test("a send held by the shared relay gate is age-fenced and recovers with an IDR request", async () => {
  let now = 10_000;
  let release;
  const requests = [];
  let publications = 0;
  const h = harness({
    now: () => now,
    capture: async (refresh) => {
      requests.push(refresh);
      return video(`frame-${requests.length}`, now);
    },
    publish: async (_event, current) => {
      publications++;
      if (publications === 1) {
        await new Promise((resolve) => {
          release = resolve;
        });
        if (!current()) throw new PublishCanceledError();
      }
    },
  });
  await flush();
  now += 600;
  release();
  await flush();
  await next(h);
  assert.equal(requests.at(-1), true);
  assert.deepEqual(h.errors, []);
  h.stop();
});

test("persistent stale native output has a terminal recovery budget", async () => {
  const h = harness({
    now: () => 20_000,
    capture: async () => video("stale", 10_000),
  });
  await flush();
  for (let index = 0; index < 7; index++) await next(h);
  assert.equal(h.errors.length, 1);
  assert.match(h.errors[0].message, /recover fresh frames/);
  assert.equal(h.events.length, 0);
  assert.equal(h.timers.length, 0);
});

test("ACK completion cannot start a second capture while a native read is pending", async () => {
  let reads = 0;
  let releaseCapture;
  let releasePublish;
  const h = harness({
    capture: async () => {
      reads++;
      if (reads === 2)
        await new Promise((resolve) => {
          releaseCapture = resolve;
        });
      return video(`frame-${reads}`, 10_000);
    },
    publish: () =>
      new Promise((resolve) => {
        releasePublish = resolve;
      }),
  });
  await flush();
  await next(h);
  releasePublish();
  await flush();
  assert.equal(reads, 2);
  assert.equal(h.timers.length, 0);
  h.stop();
  releaseCapture();
  await flush();
  assert.equal(h.timers.length, 0);
});

test("a failed wake-up reaches the native error path instead of stranding capture", async () => {
  const error = new Error("Media clock failed to schedule");
  const h = harness({
    schedule: () => {
      throw error;
    },
  });
  await flush();
  assert.deepEqual(h.errors, [error]);
  h.stop();
});
