import assert from "node:assert/strict";
import test from "node:test";

const calls = [];
globalThis.window = globalThis;
const status = {
  classification: "OFFICIAL",
  runId: "run-1",
  scheduleId: "daily-command-brief",
  state: "queued",
  updatedAt: "2026-07-25T06:00:00Z",
  degradedSections: [],
  error: null,
};
const schedule = {
  classification: "OFFICIAL",
  scheduleId: "daily-command-brief",
  enabled: true,
  localTime: "06:00",
  timezone: "Australia/Sydney",
  catchUpSameDay: true,
  concurrency: 1,
};
globalThis.__TAURI_INTERNALS__ = {
  invoke: async (command, args) => {
    calls.push({ command, args });
    if (command === "start_command_brief") return status;
    if (command === "get_command_brief_status")
      return { classification: "OFFICIAL", current: status, history: [status] };
    if (
      command === "get_command_brief_schedule" ||
      command === "set_command_brief_schedule"
    )
      return schedule;
    if (command === "cancel_command_brief") return status;
    return null;
  },
  transformCallback: () => 1,
};

const {
  cancelCommandBrief,
  getCommandBriefSchedule,
  getCommandBriefStatus,
  getLatestCommandBrief,
  setCommandBriefSchedule,
  startCommandBrief,
} = await import("./tauriCommandBrief.ts");

test("manual brief commands expose no renderer prompt, persona, tool, or endpoint input", async () => {
  calls.length = 0;
  await startCommandBrief();
  await getCommandBriefStatus();
  await getLatestCommandBrief();
  await getCommandBriefSchedule();

  assert.deepEqual(calls, [
    { command: "start_command_brief", args: {} },
    { command: "get_command_brief_status", args: {} },
    { command: "get_latest_command_brief", args: {} },
    { command: "get_command_brief_schedule", args: {} },
  ]);
});

test("cancel sends only the bounded run identity and schedule update sends only user controls", async () => {
  calls.length = 0;
  await cancelCommandBrief("run-1");
  await setCommandBriefSchedule({
    enabled: true,
    localTime: "06:00",
    concurrency: 2,
  });

  assert.deepEqual(calls, [
    { command: "cancel_command_brief", args: { runId: "run-1" } },
    {
      command: "set_command_brief_schedule",
      args: {
        update: {
          enabled: true,
          localTime: "06:00",
          concurrency: 2,
        },
      },
    },
  ]);
});
