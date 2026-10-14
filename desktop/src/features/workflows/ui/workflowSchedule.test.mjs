import assert from "node:assert/strict";
import test from "node:test";

import {
  defaultScheduleTrigger,
  isValidScheduleTimezone,
  localScheduleTimezone,
  scheduleFormFromTrigger,
  scheduleTimezoneLabel,
  scheduleTimezoneOptions,
  scheduleTriggerFromForm,
  scheduleWeekdaysFromCronField,
  updateScheduleTrigger,
} from "./workflowSchedule.ts";

function withDateTimeFormat(replacement, callback) {
  const original = Intl.DateTimeFormat;
  Intl.DateTimeFormat = replacement;
  try {
    callback();
  } finally {
    Intl.DateTimeFormat = original;
  }
}

function withResolvedTimezone(timezone, callback) {
  const NativeDateTimeFormat = Intl.DateTimeFormat;
  withDateTimeFormat(function DateTimeFormat(...args) {
    if (args.length === 0) {
      return { resolvedOptions: () => ({ timeZone: timezone }) };
    }
    return new NativeDateTimeFormat(...args);
  }, callback);
}

test("new schedules resolve the computer timezone when they are created", () => {
  for (const timezone of ["Europe/London", "America/Los_Angeles"]) {
    withResolvedTimezone(timezone, () => {
      assert.equal(localScheduleTimezone(), timezone);
      assert.deepEqual(defaultScheduleTrigger(), {
        on: "schedule",
        cron: "0 9 * * *",
        timezone,
      });
    });
  }
});

test("new schedules use UTC for unavailable, invalid, or equivalent host zones", () => {
  for (const timezone of [undefined, "", "Etc/UTC", "Not/AZone"]) {
    withResolvedTimezone(timezone, () => {
      assert.equal(localScheduleTimezone(), "UTC");
      assert.equal(defaultScheduleTrigger().timezone, "UTC");
    });
  }

  withDateTimeFormat(
    function DateTimeFormat() {
      throw new Error("timezone unavailable");
    },
    () => {
      assert.equal(localScheduleTimezone(), "UTC");
      assert.equal(defaultScheduleTrigger().timezone, "UTC");
    },
  );
});

test("timezone options include local, UTC, and imported zones once", () => {
  withResolvedTimezone("Europe/London", () => {
    assert.deepEqual(scheduleTimezoneOptions(undefined), [
      "Europe/London",
      "UTC",
    ]);
    assert.deepEqual(scheduleTimezoneOptions("Europe/London"), [
      "Europe/London",
      "UTC",
    ]);
    assert.deepEqual(scheduleTimezoneOptions("UTC"), ["Europe/London", "UTC"]);
    assert.deepEqual(scheduleTimezoneOptions("America/Chicago"), [
      "Europe/London",
      "UTC",
      "America/Chicago",
    ]);
  });
});

test("switching from elapsed intervals to cron uses local time without changing legacy cron edits", () => {
  withResolvedTimezone("Europe/London", () => {
    const daily = defaultScheduleTrigger();
    const hourly = updateScheduleTrigger(scheduleFormFromTrigger(daily), {
      frequency: "hourly",
    });
    assert.deepEqual(hourly, { on: "schedule", interval: "1h" });
    const restored = updateScheduleTrigger(scheduleFormFromTrigger(hourly), {
      frequency: "daily",
    });
    assert.equal(restored.timezone, "Europe/London");
    const custom = updateScheduleTrigger(scheduleFormFromTrigger(hourly), {
      frequency: "custom_cron",
      customCron: "",
    });
    assert.equal(custom.timezone, "Europe/London");
    const populated = updateScheduleTrigger(scheduleFormFromTrigger(custom), {
      customCron: "0 8 * * *",
    });
    assert.equal(populated.timezone, "Europe/London");
    const legacy = updateScheduleTrigger(
      scheduleFormFromTrigger({ on: "schedule", cron: "0 9 * * *" }),
      { time: "10:00" },
    );
    assert.deepEqual(legacy, { on: "schedule", cron: "0 10 * * *" });
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

test("round-trips explicit cron timezones without relabeling legacy UTC", () => {
  for (const timezone of ["America/Chicago", "UTC", "Europe/London"]) {
    const trigger = { on: "schedule", cron: "30 14 * * *", timezone };
    assert.deepEqual(
      scheduleTriggerFromForm(scheduleFormFromTrigger(trigger)),
      trigger,
    );
  }

  const legacyUtc = { on: "schedule", cron: "0 9 * * *" };
  const legacyForm = scheduleFormFromTrigger(legacyUtc);
  assert.equal(legacyForm.timezone, undefined);
  assert.deepEqual(scheduleTriggerFromForm(legacyForm), legacyUtc);
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

  const interval = scheduleTriggerFromForm({
    ...scheduleFormFromTrigger({
      on: "schedule",
      cron: "0 9 * * *",
      timezone: "America/Chicago",
    }),
    frequency: "hourly",
  });
  assert.deepEqual(interval, { on: "schedule", interval: "1h" });
  assert.equal("timezone" in interval, false);
});

test("validates IANA timezones and labels effective cron zones", () => {
  assert.equal(isValidScheduleTimezone("America/Chicago"), true);
  assert.equal(isValidScheduleTimezone("UTC"), true);
  assert.equal(isValidScheduleTimezone("Europe/London"), true);
  assert.equal(isValidScheduleTimezone(""), false);
  assert.equal(isValidScheduleTimezone(" America/Chicago"), false);
  assert.equal(isValidScheduleTimezone("Not/AZone"), false);
  assert.equal(isValidScheduleTimezone("+01:00"), false);

  assert.equal(scheduleTimezoneLabel("America/Chicago"), "Central Time");
  assert.equal(scheduleTimezoneLabel("UTC"), "UTC");
  assert.equal(scheduleTimezoneLabel(undefined), "UTC");
  assert.equal(scheduleTimezoneLabel("Europe/London"), "Europe/London");
});
