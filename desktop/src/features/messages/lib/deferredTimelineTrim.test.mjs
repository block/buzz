import assert from "node:assert/strict";
import test from "node:test";

import { createDeferredTimelineTrim } from "./deferredTimelineTrim.ts";

function fakeTimerHost() {
  let nextId = 1;
  const callbacks = new Map();
  return {
    host: {
      clearTimeout(id) {
        callbacks.delete(id);
      },
      setTimeout(callback) {
        const id = nextId++;
        callbacks.set(id, callback);
        return id;
      },
    },
    flush() {
      for (const [id, callback] of [...callbacks]) {
        callbacks.delete(id);
        callback();
      }
    },
  };
}

test("StrictMode setup cancels the synthetic cleanup trim", () => {
  const timers = fakeTimerHost();
  const scheduler = createDeferredTimelineTrim(timers.host);
  let trims = 0;

  scheduler.schedule("channel-a", () => trims++);
  scheduler.cancel("channel-a");
  timers.flush();

  assert.equal(trims, 0);
});

test("a genuine channel departure trims after the deferred task", () => {
  const timers = fakeTimerHost();
  const scheduler = createDeferredTimelineTrim(timers.host);
  let trims = 0;

  scheduler.schedule("channel-a", () => trims++);
  timers.flush();

  assert.equal(trims, 1);
});
