import assert from "node:assert/strict";
import test from "node:test";

import {
  schedulePreconnect,
  shouldBounceForChannelNotification,
} from "./AppShell.helpers.ts";

function fakeScheduler() {
  const scheduled = new Map();
  let nextHandle = 1;
  return {
    scheduler: {
      setTimeout(callback, delayMs) {
        const handle = nextHandle++;
        scheduled.set(handle, { callback, delayMs });
        return handle;
      },
      clearTimeout(handle) {
        scheduled.delete(handle);
      },
    },
    flush() {
      for (const { callback } of scheduled.values()) {
        callback();
      }
    },
    pending() {
      return [...scheduled.values()];
    },
  };
}

test("schedulePreconnect_runsOnNextTick", () => {
  const { scheduler, flush, pending } = fakeScheduler();
  let ran = 0;

  schedulePreconnect(() => ran++, scheduler);

  // Scheduled with a zero delay (next macrotask), not run synchronously.
  assert.equal(ran, 0);
  assert.equal(pending().length, 1);
  assert.equal(pending()[0].delayMs, 0);

  flush();
  assert.equal(ran, 1);
});

test("schedulePreconnect_cancelPreventsRun", () => {
  const { scheduler, flush } = fakeScheduler();
  let ran = 0;

  const cancel = schedulePreconnect(() => ran++, scheduler);
  cancel();
  flush();

  assert.equal(ran, 0);
});

test("shouldBounceForChannelNotification_allowsTopLevelChannelMessages", () => {
  assert.equal(shouldBounceForChannelNotification([["h", "channel"]]), true);
});

test("shouldBounceForChannelNotification_suppressesThreadReplies", () => {
  assert.equal(
    shouldBounceForChannelNotification([
      ["h", "channel"],
      ["e", "root", "", "reply"],
    ]),
    false,
  );
});

test("shouldBounceForChannelNotification_allowsBroadcastReplies", () => {
  assert.equal(
    shouldBounceForChannelNotification([
      ["h", "channel"],
      ["e", "root", "", "reply"],
      ["broadcast", "1"],
    ]),
    true,
  );
});
