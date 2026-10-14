import assert from "node:assert/strict";
import test from "node:test";

import {
  getAudioMediaLoadSchedulerSnapshot,
  MAX_CONCURRENT_AUDIO_MEDIA_LOADS,
  resetAudioMediaLoadScheduler,
  scheduleAudioMediaLoad,
} from "./audioMediaLoadScheduler.ts";

function abortablePendingTask(onStart) {
  return (signal) =>
    new Promise((resolve, reject) => {
      onStart({ resolve, signal });
      signal.addEventListener(
        "abort",
        () => reject(new DOMException("cancelled", "AbortError")),
        { once: true },
      );
    });
}

test.afterEach(async () => {
  resetAudioMediaLoadScheduler();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(getAudioMediaLoadSchedulerSnapshot(), {
    active: 0,
    queued: 0,
  });
});

test("hard-caps active work and removes queued work on cancellation", async () => {
  const starts = [];
  const handles = Array.from({ length: 7 }, () =>
    scheduleAudioMediaLoad(abortablePendingTask((start) => starts.push(start))),
  );
  const settlements = handles.map((handle) =>
    handle.promise.catch((error) => error),
  );

  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(starts.length, MAX_CONCURRENT_AUDIO_MEDIA_LOADS);
  assert.deepEqual(getAudioMediaLoadSchedulerSnapshot(), {
    active: MAX_CONCURRENT_AUDIO_MEDIA_LOADS,
    queued: 7 - MAX_CONCURRENT_AUDIO_MEDIA_LOADS,
  });

  for (const handle of handles) handle.cancel();
  await Promise.all(settlements);
  assert.equal(starts.length, MAX_CONCURRENT_AUDIO_MEDIA_LOADS);
  assert.ok(starts.every(({ signal }) => signal.aborted));
  assert.deepEqual(getAudioMediaLoadSchedulerSnapshot(), {
    active: 0,
    queued: 0,
  });
});

test("settled work promotes only enough queued work to refill the cap", async () => {
  const starts = [];
  const handles = Array.from({ length: 5 }, () =>
    scheduleAudioMediaLoad(abortablePendingTask((start) => starts.push(start))),
  );
  const settlements = handles.map((handle) =>
    handle.promise.catch((error) => error),
  );

  await new Promise((resolve) => setTimeout(resolve, 0));
  starts[0].resolve("first");
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(starts.length, MAX_CONCURRENT_AUDIO_MEDIA_LOADS + 1);
  assert.deepEqual(getAudioMediaLoadSchedulerSnapshot(), {
    active: MAX_CONCURRENT_AUDIO_MEDIA_LOADS,
    queued: 1,
  });

  for (const handle of handles) handle.cancel();
  await Promise.all(settlements);
});
