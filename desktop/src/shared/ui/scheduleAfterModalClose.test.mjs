import assert from "node:assert/strict";
import { afterEach, describe, it } from "node:test";

import { MODAL_CLOSE_DURATION_MS } from "./modalMotion.ts";
import { scheduleAfterModalClose } from "./scheduleAfterModalClose.ts";

const originalWindow = globalThis.window;

afterEach(() => {
  if (originalWindow === undefined) delete globalThis.window;
  else globalThis.window = originalWindow;
});

function installTimeoutStub() {
  const tasks = new Map();
  let nextId = 0;
  globalThis.window = {
    setTimeout(callback, delay) {
      const id = ++nextId;
      tasks.set(id, { callback, delay });
      return id;
    },
    clearTimeout(id) {
      tasks.delete(id);
    },
  };
  return {
    flush(id) {
      const entry = tasks.get(id);
      assert.ok(entry, `missing timeout ${id}`);
      tasks.delete(id);
      entry.callback();
    },
    delay(id) {
      return tasks.get(id)?.delay;
    },
    pendingCount() {
      return tasks.size;
    },
  };
}

describe("scheduleAfterModalClose", () => {
  it("does not run the task in the same turn as catalog close", () => {
    const timeouts = installTimeoutStub();
    let ran = false;
    scheduleAfterModalClose(() => {
      ran = true;
    });
    assert.equal(ran, false);
    assert.equal(timeouts.pendingCount(), 1);
  });

  it("waits the closed-dialog duration, then runs", () => {
    const timeouts = installTimeoutStub();
    let ran = false;
    const cancel = scheduleAfterModalClose(() => {
      ran = true;
    });
    assert.equal(timeouts.delay(1), MODAL_CLOSE_DURATION_MS);
    timeouts.flush(1);
    assert.equal(ran, true);
    cancel();
  });

  it("cancel prevents a late import after leaving Agents", () => {
    const timeouts = installTimeoutStub();
    let ran = false;
    const cancel = scheduleAfterModalClose(() => {
      ran = true;
    });
    cancel();
    assert.equal(timeouts.pendingCount(), 0);
    assert.equal(ran, false);
  });
});
