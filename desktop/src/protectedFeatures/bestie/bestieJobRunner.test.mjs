import assert from "node:assert/strict";
import test from "node:test";

import {
  addBestieJob,
  emptyBestieJobState,
  markBestieJobFired,
} from "./bestieJobStorage.ts";
import { startBestieJobRunner } from "./bestieJobRunner.ts";

test("job runner fires due once and reschedules recurring", () => {
  const timers = [];
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;
  const originalSetInterval = globalThis.setInterval;
  const originalClearInterval = globalThis.clearInterval;
  globalThis.window = globalThis;
  globalThis.setTimeout = (fn, ms) => {
    const id = timers.length + 1;
    timers.push({ fn, ms, type: "timeout" });
    return id;
  };
  globalThis.clearTimeout = () => {};
  globalThis.setInterval = (fn) => {
    timers.push({ fn, type: "interval" });
    return 99;
  };
  globalThis.clearInterval = () => {};

  let state = emptyBestieJobState();
  state = addBestieJob(
    state,
    {
      prompt: "Do it",
      schedule: { everySeconds: 120, kind: "interval" },
      title: "Do",
    },
    1000,
  );
  // Make due now
  state = {
    ...state,
    jobs: state.jobs.map((j) => ({ ...j, nextDueAt: 1000 })),
  };

  let fires = 0;
  const handles = startBestieJobRunner({
    getJobState: () => state,
    intervalMs: 60_000,
    nowSeconds: () => 1000,
    onDueJobs: (jobs) => {
      fires += jobs.length;
      for (const job of jobs) {
        const next = markBestieJobFired(state, job.id, job.nextDueAt, 1000);
        if (next) state = next;
      }
    },
  });

  assert.equal(fires, 1);
  // Second tick should not re-fire same slot
  handles.tick();
  assert.equal(fires, 1);
  assert.equal(state.jobs[0].nextDueAt, 1120);
  handles.stop();

  globalThis.setTimeout = originalSetTimeout;
  globalThis.clearTimeout = originalClearTimeout;
  globalThis.setInterval = originalSetInterval;
  globalThis.clearInterval = originalClearInterval;
});
