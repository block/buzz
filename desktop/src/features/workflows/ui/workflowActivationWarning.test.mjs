import assert from "node:assert/strict";
import test from "node:test";

import { getWorkflowActivationWarning } from "./workflowActivationWarning.ts";

function workflowYaml(trigger) {
  return `name: Test\ntrigger:\n${trigger}\nsteps:\n  - id: notify\n    action: send_message\n    text: Hi\n`;
}

test("warns when a message trigger matches every message", () => {
  assert.deepEqual(
    getWorkflowActivationWarning(workflowYaml("  on: message_posted")),
    {
      description:
        "It will run for every new message in this channel. Review the trigger before turning it on.",
      title: "This workflow may run often",
    },
  );
});

test("does not warn when a message trigger is narrowed", () => {
  assert.equal(
    getWorkflowActivationWarning(
      workflowYaml('  on: message_posted\n  filter: text == "deploy"'),
    ),
    null,
  );
});

test("warns for hourly-or-faster interval schedules with concrete copy", () => {
  assert.equal(
    getWorkflowActivationWarning(
      workflowYaml("  on: schedule\n  interval: 15m"),
    )?.description,
    "It is scheduled to run every 15 minutes. Review the schedule before turning it on.",
  );
  assert.equal(
    getWorkflowActivationWarning(
      workflowYaml("  on: schedule\n  interval: 2h"),
    ),
    null,
  );
});

test("warns for clear hourly-or-faster cron schedules", () => {
  for (const cron of ["*/5 * * * *", "0 */5 * * * *", "0 */5 * * * * *"]) {
    assert.equal(
      getWorkflowActivationWarning(
        workflowYaml(`  on: schedule\n  cron: "${cron}"`),
      )?.description,
      "It is scheduled to run every 5 minutes. Review the schedule before turning it on.",
    );
  }
  assert.equal(
    getWorkflowActivationWarning(
      workflowYaml('  on: schedule\n  cron: "*/10 * * * * *"'),
    )?.description,
    "It is scheduled to run multiple times a minute. Review the schedule before turning it on.",
  );
  assert.equal(
    getWorkflowActivationWarning(
      workflowYaml('  on: schedule\n  cron: "0 9 * * *"'),
    ),
    null,
  );
});

test("warns for stepped, ranged, and listed hourly cron schedules", () => {
  for (const cron of [
    "0 0 */1 * * *",
    "0 0 * * * *",
    "0 0 0-23 * * *",
    "0 0 0-11,12-23 * * *",
    "0 0 */1 * * * *",
  ]) {
    assert.equal(
      getWorkflowActivationWarning(
        workflowYaml(`  on: schedule\n  cron: "${cron}"`),
      )?.description,
      "It is scheduled to run every hour. Review the schedule before turning it on.",
      cron,
    );
  }
  assert.equal(
    getWorkflowActivationWarning(
      workflowYaml('  on: schedule\n  cron: "*/5 0-23 * * *"'),
    )?.description,
    "It is scheduled to run every 5 minutes. Review the schedule before turning it on.",
  );
  for (const cron of ["0 */3 * * *", "0 0 0,12 * * *", "0 0 8-17 * * *"]) {
    assert.equal(
      getWorkflowActivationWarning(
        workflowYaml(`  on: schedule\n  cron: "${cron}"`),
      ),
      null,
      cron,
    );
  }
});

test("returns no warning for malformed or unrelated definitions", () => {
  assert.equal(getWorkflowActivationWarning("not: [valid"), null);
  assert.equal(
    getWorkflowActivationWarning(workflowYaml("  on: reaction_added")),
    null,
  );
});
