import assert from "node:assert/strict";
import test from "node:test";

const calls = [];
globalThis.window = globalThis;
globalThis.__TAURI_INTERNALS__ = {
  invoke: async (command, args) => {
    calls.push({ command, args });
    return {
      source: "calendar",
      permission: "not_determined",
      observedAt: "2026-07-24T00:00:00Z",
      records: [],
      truncated: false,
      error: null,
    };
  },
  transformCallback: () => 1,
};

const { readAppleInputs } = await import("./tauriAppleInputs.ts");

test("readAppleInputs invokes the closed native request boundary unchanged", async () => {
  calls.length = 0;
  const request = {
    operation: "read_calendar",
    arguments: {
      calendar_ids: ["work"],
      start: "2026-07-24T00:00:00Z",
      end: "2026-07-25T00:00:00Z",
      maximum: 25,
    },
  };

  const result = await readAppleInputs(request);

  assert.deepEqual(calls, [
    {
      command: "read_apple_inputs",
      args: { request },
    },
  ]);
  assert.equal(result.source, "calendar");
  assert.equal(result.permission, "not_determined");
});
