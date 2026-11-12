import assert from "node:assert/strict";
import { afterEach, describe, it } from "node:test";

import { scheduleAfterForegroundReady } from "./foregroundReady.ts";

const originalWindow = globalThis.window;

afterEach(() => {
  if (originalWindow === undefined) delete globalThis.window;
  else globalThis.window = originalWindow;
});

function installSchedulerStub({ animationFrame = true } = {}) {
  const tasks = new Map();
  const frames = new Map();
  let nextId = 0;
  globalThis.window = {
    setTimeout(callback) {
      const id = ++nextId;
      tasks.set(id, callback);
      return id;
    },
    clearTimeout(id) {
      tasks.delete(id);
    },
    ...(animationFrame
      ? {
          requestAnimationFrame(callback) {
            const id = ++nextId;
            frames.set(id, callback);
            return id;
          },
          cancelAnimationFrame(id) {
            frames.delete(id);
          },
        }
      : {}),
  };
  const runFirst = (queue, argument) => {
    const entry = queue.entries().next().value;
    assert.ok(entry, "expected queued work");
    const [id, callback] = entry;
    queue.delete(id);
    callback(argument);
  };
  return {
    frames,
    tasks,
    runTask: () => runFirst(tasks),
    runFrame: () => runFirst(frames, 0),
  };
}

describe("scheduleAfterForegroundReady", () => {
  it("runs after task, animation frame, and trailing task", () => {
    const stub = installSchedulerStub();
    let calls = 0;

    scheduleAfterForegroundReady(() => calls++);
    assert.equal(calls, 0);
    stub.runTask();
    assert.equal(calls, 0);
    stub.runFrame();
    assert.equal(calls, 0);
    stub.runTask();
    assert.equal(calls, 1);
  });

  it("falls back to two tasks when animation frames are unavailable", () => {
    const stub = installSchedulerStub({ animationFrame: false });
    let calls = 0;

    scheduleAfterForegroundReady(() => calls++);
    stub.runTask();
    assert.equal(calls, 0);
    stub.runTask();
    assert.equal(calls, 1);
  });

  it("cancels work at every scheduling stage", () => {
    for (const cancelAfter of ["schedule", "task", "frame"]) {
      const stub = installSchedulerStub();
      let calls = 0;
      const cancel = scheduleAfterForegroundReady(() => calls++);
      if (cancelAfter !== "schedule") stub.runTask();
      if (cancelAfter === "frame") stub.runFrame();
      cancel();

      assert.equal(stub.tasks.size, 0, `${cancelAfter}: tasks cleared`);
      assert.equal(stub.frames.size, 0, `${cancelAfter}: frames cleared`);
      assert.equal(calls, 0, `${cancelAfter}: callback suppressed`);
    }
  });
});
