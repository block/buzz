import assert from "node:assert/strict";
import test from "node:test";

import {
  addBestieJob,
  dueBestieJobs,
  emptyBestieJobState,
  markBestieJobFired,
} from "./bestieJobStorage.ts";

test("due job fires once then reschedules interval", () => {
  let state = emptyBestieJobState();
  state = addBestieJob(
    state,
    {
      prompt: "Check inbox",
      schedule: { everySeconds: 120, kind: "interval" },
      title: "Inbox",
    },
    1000,
  );
  assert.equal(state.jobs[0].nextDueAt, 1120);
  // Pretend due now
  state = {
    ...state,
    jobs: state.jobs.map((j) => ({ ...j, nextDueAt: 1000 })),
  };
  const due = dueBestieJobs(state, 1000);
  assert.equal(due.length, 1);
  const after = markBestieJobFired(state, due[0].id, 1000, 1000);
  assert.ok(after);
  assert.equal(dueBestieJobs(after, 1000).length, 0);
  assert.equal(after.jobs[0].lastRunAt, 1000);
  assert.equal(after.jobs[0].nextDueAt, 1120);
  // Same slot again is idempotent
  assert.equal(markBestieJobFired(after, due[0].id, 1000, 1001), null);
});

test("once job disables after fire", () => {
  let state = emptyBestieJobState();
  state = addBestieJob(
    state,
    {
      prompt: "One shot",
      schedule: { dueAt: 500, kind: "once" },
      title: "Once",
    },
    100,
  );
  const due = dueBestieJobs(state, 500);
  assert.equal(due.length, 1);
  const after = markBestieJobFired(state, due[0].id, 500, 500);
  assert.equal(after.jobs[0].enabled, false);
  assert.equal(after.jobs[0].nextDueAt, null);
  assert.equal(dueBestieJobs(after, 600).length, 0);
});
