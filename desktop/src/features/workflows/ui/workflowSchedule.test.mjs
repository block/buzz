import assert from "node:assert/strict";
import test from "node:test";

import {
  defaultScheduleTrigger,
  scheduleFormFromTrigger,
  scheduleTriggerFromForm,
  scheduleWeekdaysFromCronField,
} from "./workflowSchedule.ts";

test("new schedules default to daily at 09:00 UTC", () => {
  assert.deepEqual(defaultScheduleTrigger(), {
    on: "schedule",
    cron: "0 9 * * *",
  });
});

test("recognizes the 15m, 30m, and hourly presets", () => {
  assert.equal(
    scheduleFormFromTrigger({ on: "schedule", interval: "15m" }).frequency,
    "every_15_minutes",
  );
  assert.equal(
    scheduleFormFromTrigger({ on: "schedule", interval: "30m" }).frequency,
    "every_30_minutes",
  );
  assert.equal(
    scheduleFormFromTrigger({ on: "schedule", interval: "1h" }).frequency,
    "hourly",
  );
});

test("round-trips daily, weekly, monthly, and custom cron schedules", () => {
  for (const cron of [
    "30 14 * * *",
    "30 14 * * 1-5",
    "30 14 * * 1,3,5",
    "30 14 * * 4",
    "30 14 23 * *",
    "0 */2 * * 1,3,5",
  ]) {
    const form = scheduleFormFromTrigger({ on: "schedule", cron });
    assert.deepEqual(scheduleTriggerFromForm(form), { on: "schedule", cron });
  }
});

test("preserves arbitrary custom cron and legacy interval strings exactly", () => {
  const cron = " 0 */2 * * 1,3,5 ";
  const cronForm = scheduleFormFromTrigger({ on: "schedule", cron });
  assert.equal(cronForm.frequency, "custom_cron");
  assert.equal(cronForm.customCron, cron);
  assert.deepEqual(scheduleTriggerFromForm(cronForm), {
    on: "schedule",
    cron,
  });

  const interval = "2h30m";
  const intervalForm = scheduleFormFromTrigger({ on: "schedule", interval });
  assert.equal(intervalForm.frequency, "custom_interval");
  assert.equal(intervalForm.customInterval, interval);
  assert.deepEqual(scheduleTriggerFromForm(intervalForm), {
    on: "schedule",
    interval,
  });
});

test("expands numeric weekday lists and ranges for the weekly picker", () => {
  assert.deepEqual(scheduleWeekdaysFromCronField("1-5"), [
    "1",
    "2",
    "3",
    "4",
    "5",
  ]);
  assert.deepEqual(scheduleWeekdaysFromCronField("1,3,5"), ["1", "3", "5"]);
  assert.deepEqual(scheduleWeekdaysFromCronField("MON-FRI"), []);
});

test("switching schedule modes never emits cron and interval together", () => {
  const form = scheduleFormFromTrigger({ on: "schedule", interval: "30m" });
  const custom = scheduleTriggerFromForm({
    ...form,
    customCron: "0 8 * * 6",
    frequency: "custom_cron",
  });

  assert.deepEqual(custom, { on: "schedule", cron: "0 8 * * 6" });
  assert.equal("interval" in custom, false);
});
