import assert from "node:assert/strict";
import test from "node:test";

import { PresenceSubscriptionReconciler } from "./presenceSubscriptionReconciler.ts";

const A = "a".repeat(64);
const B = "b".repeat(64);
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

test("normalizes demand and refuses to open an empty subscription", async () => {
  const opened = [];
  const reconciler = new PresenceSubscriptionReconciler({
    open: async (authors) => {
      opened.push(authors);
      return async () => {};
    },
  });

  reconciler.setAuthors([]);
  reconciler.setAuthors([B, A.toUpperCase(), A]);
  await flush();
  assert.deepEqual(opened, [[A, B]]);
  reconciler.dispose();
});

test("opens replacement before closing the previous subscription", async () => {
  const actions = [];
  const reconciler = new PresenceSubscriptionReconciler({
    open: async (authors) => {
      const key = authors.join("");
      actions.push(`open:${key}`);
      return async () => actions.push(`close:${key}`);
    },
  });

  reconciler.setAuthors([A]);
  await flush();
  reconciler.setAuthors([B]);
  await flush();
  assert.deepEqual(actions, [`open:${A}`, `open:${B}`, `close:${A}`]);
  reconciler.dispose();
});

test("a stale async open is closed and never installed", async () => {
  const first = deferred();
  const actions = [];
  let opens = 0;
  const reconciler = new PresenceSubscriptionReconciler({
    open: async (authors) => {
      opens += 1;
      const key = authors.join("");
      actions.push(`open:${key}`);
      if (opens === 1) await first.promise;
      return async () => actions.push(`close:${key}`);
    },
  });

  reconciler.setAuthors([A]);
  await flush();
  reconciler.setAuthors([B]);
  first.resolve();
  await flush();
  await flush();
  assert.deepEqual(actions, [`open:${A}`, `close:${A}`, `open:${B}`]);
  reconciler.dispose();
});

test("default timers preserve their global receiver when scheduling retries", async () => {
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;
  const timers = [];
  const cleared = [];

  globalThis.setTimeout = function (callback) {
    assert.equal(this, globalThis);
    timers.push(callback);
    return timers.length;
  };
  globalThis.clearTimeout = function (timer) {
    assert.equal(this, globalThis);
    cleared.push(timer);
  };

  try {
    const reconciler = new PresenceSubscriptionReconciler({
      open: async () => {
        throw new Error("relay unavailable");
      },
    });

    reconciler.setAuthors([A]);
    await Promise.resolve();
    assert.equal(timers.length, 1);

    reconciler.dispose();
    assert.deepEqual(cleared, [1]);
  } finally {
    globalThis.setTimeout = originalSetTimeout;
    globalThis.clearTimeout = originalClearTimeout;
  }
});

test("failed replacement preserves the previous subscription and retries", async () => {
  const timers = [];
  const actions = [];
  let failB = true;
  const reconciler = new PresenceSubscriptionReconciler({
    open: async (authors) => {
      const key = authors.join("");
      actions.push(`open:${key}`);
      if (key === B && failB) throw new Error("relay unavailable");
      return async () => actions.push(`close:${key}`);
    },
    retryDelay: () => 1,
    setTimer: (callback) => {
      timers.push(callback);
      return timers.length;
    },
    clearTimer: () => {},
  });

  reconciler.setAuthors([A]);
  await flush();
  reconciler.setAuthors([B]);
  await flush();
  assert.deepEqual(actions, [`open:${A}`, `open:${B}`]);
  assert.equal(timers.length, 1);

  failB = false;
  timers.shift()();
  await flush();
  assert.deepEqual(actions, [
    `open:${A}`,
    `open:${B}`,
    `open:${B}`,
    `close:${A}`,
  ]);
  reconciler.dispose();
});

test("default browser timers retry safely after an open failure and cancel pending work", async () => {
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;
  const timers = [];
  const cleared = [];
  const opened = [];
  let shouldFail = true;
  const setTimerWithGlobalReceiver = function (callback, delayMs) {
    assert.equal(this, globalThis);
    const timer = { callback, delayMs };
    timers.push(timer);
    return timer;
  };
  const clearTimerWithGlobalReceiver = function (timer) {
    assert.equal(this, globalThis);
    cleared.push(timer);
  };

  const reconciler = new PresenceSubscriptionReconciler({
    open: async (authors) => {
      opened.push(authors);
      if (shouldFail) throw new Error("rate-limited: retry later");
      return async () => {};
    },
    retryDelay: () => 25,
  });

  try {
    globalThis.setTimeout = setTimerWithGlobalReceiver;
    reconciler.setAuthors([A]);
    await Promise.resolve();
    await Promise.resolve();
    globalThis.setTimeout = originalSetTimeout;

    assert.equal(timers.length, 1);
    assert.equal(timers[0].delayMs, 25);

    shouldFail = false;
    timers[0].callback();
    await Promise.resolve();
    await Promise.resolve();
    assert.deepEqual(opened, [[A], [A]]);

    shouldFail = true;
    globalThis.setTimeout = setTimerWithGlobalReceiver;
    reconciler.setAuthors([B]);
    await Promise.resolve();
    await Promise.resolve();
    globalThis.setTimeout = originalSetTimeout;

    globalThis.clearTimeout = clearTimerWithGlobalReceiver;
    reconciler.dispose();
    globalThis.clearTimeout = originalClearTimeout;
    assert.deepEqual(cleared, [timers[1]]);
  } finally {
    globalThis.setTimeout = originalSetTimeout;
    globalThis.clearTimeout = originalClearTimeout;
  }
});

test("rapid A to B to C keeps A until C is confirmed", async () => {
  const openingB = deferred();
  const actions = [];
  const reconciler = new PresenceSubscriptionReconciler({
    open: async (authors) => {
      const key = authors.join("");
      actions.push(`open:${key}`);
      if (key === B) await openingB.promise;
      return async () => actions.push(`close:${key}`);
    },
  });

  reconciler.setAuthors([A]);
  await flush();
  reconciler.setAuthors([B]);
  await flush();
  reconciler.setAuthors(["c".repeat(64)]);
  openingB.resolve();
  await flush();
  await flush();
  assert.deepEqual(actions, [
    `open:${A}`,
    `open:${B}`,
    `close:${B}`,
    `open:${"c".repeat(64)}`,
    `close:${A}`,
  ]);
  reconciler.dispose();
});

test("rapid A to B to A closes stale B but retains current A", async () => {
  const openingB = deferred();
  const actions = [];
  const reconciler = new PresenceSubscriptionReconciler({
    open: async (authors) => {
      const key = authors.join("");
      actions.push(`open:${key}`);
      if (key === B) await openingB.promise;
      return async () => actions.push(`close:${key}`);
    },
  });

  reconciler.setAuthors([A]);
  await flush();
  reconciler.setAuthors([B]);
  await flush();
  reconciler.setAuthors([A]);
  openingB.resolve();
  await flush();
  assert.deepEqual(actions, [`open:${A}`, `open:${B}`, `close:${B}`]);
  reconciler.dispose();
});

test("prior close failure does not unseat a confirmed replacement", async () => {
  const actions = [];
  const reconciler = new PresenceSubscriptionReconciler({
    open: async (authors) => {
      const key = authors.join("");
      actions.push(`open:${key}`);
      return async () => {
        actions.push(`close:${key}`);
        if (key === A) throw new Error("close failed");
      };
    },
  });

  reconciler.setAuthors([A]);
  await flush();
  reconciler.setAuthors([B]);
  await flush();
  reconciler.setAuthors([B]);
  await flush();
  assert.deepEqual(actions, [`open:${A}`, `open:${B}`, `close:${A}`]);
  reconciler.dispose();
});

test("dispose closes current and a late replacement without double-closing current", async () => {
  const openingB = deferred();
  const closes = { [A]: 0, [B]: 0 };
  const reconciler = new PresenceSubscriptionReconciler({
    open: async (authors) => {
      const key = authors.join("");
      if (key === B) await openingB.promise;
      return async () => {
        closes[key] += 1;
      };
    },
  });

  reconciler.setAuthors([A]);
  await flush();
  reconciler.setAuthors([B]);
  await flush();
  reconciler.dispose();
  openingB.resolve();
  await flush();
  assert.deepEqual(closes, { [A]: 1, [B]: 1 });
});

test("dispose during an in-flight open closes the late subscription", async () => {
  const opening = deferred();
  let closeCount = 0;
  const reconciler = new PresenceSubscriptionReconciler({
    open: async () => {
      await opening.promise;
      return async () => {
        closeCount += 1;
      };
    },
  });

  reconciler.setAuthors([A]);
  await flush();
  reconciler.dispose();
  opening.resolve();
  await flush();
  assert.equal(closeCount, 1);
});

test("clearing demand closes current without opening a replacement", async () => {
  const actions = [];
  const reconciler = new PresenceSubscriptionReconciler({
    open: async (authors) => {
      const key = authors.join("");
      actions.push(`open:${key}`);
      return async () => actions.push(`close:${key}`);
    },
  });

  reconciler.setAuthors([A]);
  await flush();
  reconciler.setAuthors([]);
  await flush();
  assert.deepEqual(actions, [`open:${A}`, `close:${A}`]);
  reconciler.dispose();
});
