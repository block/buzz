import assert from "node:assert/strict";
import test from "node:test";

import {
  LocalRunCancelledError,
  LocalRunScheduler,
  MAX_LOCAL_RUN_TASK_ID_BYTES,
} from "./localRunScheduler.ts";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, reject, resolve };
}

test("runs sequentially by default and preserves FIFO fairness", async () => {
  const scheduler = new LocalRunScheduler();
  const firstGate = deferred();
  const starts = [];

  const first = scheduler.enqueue("first", async () => {
    starts.push("first");
    await firstGate.promise;
    return 1;
  });
  const second = scheduler.enqueue("second", () => {
    starts.push("second");
    return 2;
  });
  const third = scheduler.enqueue("third", async () => {
    starts.push("third");
    return 3;
  });

  await Promise.resolve();
  assert.deepEqual(starts, ["first"]);
  firstGate.resolve();

  assert.equal(await first.result, 1);
  assert.equal(await second.result, 2);
  assert.equal(await third.result, 3);
  assert.deepEqual(starts, ["first", "second", "third"]);
});

test("permits capacity two and rejects capacity outside one to two", async () => {
  assert.throws(() => new LocalRunScheduler({ capacity: 0 }), RangeError);
  assert.throws(() => new LocalRunScheduler({ capacity: 3 }), RangeError);
  assert.throws(() => new LocalRunScheduler({ capacity: 1.5 }), RangeError);

  const scheduler = new LocalRunScheduler({ capacity: 2 });
  const gate = deferred();
  const starts = [];
  const first = scheduler.enqueue("first", async () => {
    starts.push("first");
    await gate.promise;
  });
  const second = scheduler.enqueue("second", async () => {
    starts.push("second");
    await gate.promise;
  });
  const third = scheduler.enqueue("third", () => {
    starts.push("third");
  });

  await Promise.resolve();
  assert.deepEqual(starts, ["first", "second"]);
  gate.resolve();
  await Promise.all([first.result, second.result, third.result]);
  assert.deepEqual(starts, ["first", "second", "third"]);
});

test("cancels queued work without starting it and continues the queue", async () => {
  const scheduler = new LocalRunScheduler();
  const gate = deferred();
  let cancelledStarted = false;
  const first = scheduler.enqueue("first", () => gate.promise);
  const cancelled = scheduler.enqueue("cancelled", () => {
    cancelledStarted = true;
  });
  const third = scheduler.enqueue("third", () => "third-result");

  assert.equal(cancelled.cancel(), true);
  assert.equal(cancelled.cancel(), false);
  await assert.rejects(cancelled.result, LocalRunCancelledError);
  gate.resolve();

  await first.result;
  assert.equal(await third.result, "third-result");
  assert.equal(cancelledStarted, false);
});

test("signals running cancellation, rejects its result, and waits for task settlement", async () => {
  const scheduler = new LocalRunScheduler();
  const underlyingGate = deferred();
  const starts = [];
  let observedAbort = false;
  const running = scheduler.enqueue("running", async (signal) => {
    starts.push("running");
    signal.addEventListener("abort", () => {
      observedAbort = true;
    });
    await underlyingGate.promise;
    return "ignored";
  });
  const next = scheduler.enqueue("next", () => {
    starts.push("next");
    return "next";
  });

  await Promise.resolve();
  assert.equal(running.cancel(), true);
  await assert.rejects(running.result, LocalRunCancelledError);
  assert.equal(observedAbort, true);
  assert.deepEqual(starts, ["running"]);

  underlyingGate.resolve();
  assert.equal(await next.result, "next");
  assert.deepEqual(starts, ["running", "next"]);
});

test("isolates synchronous throws and async rejections without starving later work", async () => {
  const scheduler = new LocalRunScheduler();
  const syncFailure = new Error("sync failure");
  const asyncFailure = new Error("async failure");
  const sync = scheduler.enqueue("sync", () => {
    throw syncFailure;
  });
  const asyncRun = scheduler.enqueue("async", async () => {
    throw asyncFailure;
  });
  const success = scheduler.enqueue("success", () => 42);

  await assert.rejects(sync.result, syncFailure);
  await assert.rejects(asyncRun.result, asyncFailure);
  assert.equal(await success.result, 42);
});

test("handles reentrant enqueue without exceeding capacity or losing FIFO order", async () => {
  const scheduler = new LocalRunScheduler();
  const starts = [];
  let nested;
  const first = scheduler.enqueue("first", () => {
    starts.push("first");
    nested = scheduler.enqueue("nested", () => {
      starts.push("nested");
      return "nested";
    });
    return "first";
  });
  const second = scheduler.enqueue("second", () => {
    starts.push("second");
    return "second";
  });

  assert.equal(await first.result, "first");
  assert.equal(await second.result, "second");
  assert.equal(await nested.result, "nested");
  assert.deepEqual(starts, ["first", "second", "nested"]);
});

test("returns typed task IDs and rejects duplicate live IDs", async () => {
  const scheduler = new LocalRunScheduler();
  const gate = deferred();
  const first = scheduler.enqueue("same-id", () => gate.promise);

  assert.equal(first.taskId, "same-id");
  assert.throws(
    () => scheduler.enqueue("same-id", () => "duplicate"),
    /already queued or running/,
  );
  assert.throws(() => scheduler.enqueue("", () => "invalid"), TypeError);

  gate.resolve("result");
  assert.equal(await first.result, "result");
  const reused = scheduler.enqueue("same-id", () => "reused");
  assert.equal(await reused.result, "reused");
});

test("requires trimmed control-free task IDs within a 256-byte UTF-8 cap", async () => {
  assert.equal(MAX_LOCAL_RUN_TASK_ID_BYTES, 256);
  const scheduler = new LocalRunScheduler();

  for (const taskId of [
    "",
    " ",
    " leading",
    "trailing ",
    "line\nbreak",
    "nul\u0000byte",
    "delete\u007fbyte",
    "a".repeat(257),
    `${"é".repeat(128)}a`,
  ]) {
    assert.throws(() => scheduler.enqueue(taskId, () => taskId), TypeError);
  }

  const ascii = scheduler.enqueue("a".repeat(256), () => "ascii");
  assert.equal(await ascii.result, "ascii");
  const multibyte = scheduler.enqueue("é".repeat(128), () => "multibyte");
  assert.equal(await multibyte.result, "multibyte");
});

test("does not produce unhandled rejections when cancelled work later rejects", async () => {
  const scheduler = new LocalRunScheduler();
  const gate = deferred();
  const unhandled = [];
  const listener = (reason) => unhandled.push(reason);
  process.on("unhandledRejection", listener);
  try {
    const run = scheduler.enqueue("cancel-late-reject", () => gate.promise);
    await Promise.resolve();
    run.cancel();
    await assert.rejects(run.result, LocalRunCancelledError);
    gate.reject(new Error("late task failure"));
    await new Promise((resolve) => setImmediate(resolve));
    assert.deepEqual(unhandled, []);
  } finally {
    process.off("unhandledRejection", listener);
  }
});

test("internally observes a rejected result when the caller intentionally ignores it", async () => {
  const scheduler = new LocalRunScheduler();
  const unhandled = [];
  const listener = (reason) => unhandled.push(reason);
  process.on("unhandledRejection", listener);
  try {
    scheduler.enqueue("ignored-failure", () => {
      throw new Error("ignored by caller");
    });
    await new Promise((resolve) => setImmediate(resolve));
    assert.deepEqual(unhandled, []);
  } finally {
    process.off("unhandledRejection", listener);
  }
});
