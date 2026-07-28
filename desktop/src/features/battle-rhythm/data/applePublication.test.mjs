import assert from "node:assert/strict";
import test from "node:test";

globalThis.window = globalThis;
const calls = [];
let response;
globalThis.__TAURI_INTERNALS__ = {
  invoke: async (command, args) => {
    calls.push({ command, args });
    return response;
  },
  transformCallback: () => 1,
};

const {
  parseApplePublicationStatus,
  projectBattleRhythmToApple,
  publishBattleRhythmToApple,
} = await import("./applePublication.ts");

function event(overrides = {}) {
  return {
    schemaVersion: 1,
    id: "brief",
    ownership: { kind: "manual" },
    title: "Navigation brief",
    description: null,
    type: "brief",
    start: "2026-07-29T08:00:00+10:00",
    end: "2026-07-29T08:30:00+10:00",
    allDay: false,
    timeZone: "Australia/Sydney",
    status: "approved",
    location: "Bridge",
    responsibleOwner: "Navigator",
    participants: ["CO"],
    remarks: null,
    linkedPlanId: null,
    linkedTaskId: null,
    linkedMissionRequirementId: null,
    parentActivityId: null,
    recurrence: null,
    excludedOccurrenceStarts: [],
    ...overrides,
  };
}

test("projects only approved events with stable Battle Rhythm IDs", () => {
  const projected = projectBattleRhythmToApple([
    event(),
    event({ id: "draft", status: "draft" }),
  ]);

  assert.equal(projected.length, 1);
  assert.equal(projected[0].external_id, "battle-rhythm:brief");
  assert.match(projected[0].notes, /Responsible: Navigator/);
});

test("publishes the authoritative coverage and parses reconciliation counts", async () => {
  calls.length = 0;
  response = {
    source: "calendar",
    permission: "authorized",
    observedAt: "2026-07-29T00:00:00Z",
    records: [
      {
        fields: {
          calendar_identifier: "dedicated",
          created: "1",
          updated: "0",
          deleted: "0",
          unchanged: "0",
        },
      },
    ],
    truncated: false,
    error: null,
  };

  const result = await publishBattleRhythmToApple([event()], {
    start: "2026-01-01T00:00:00+11:00",
    end: "2028-01-01T00:00:00+11:00",
  });

  assert.equal(result.state, "published");
  assert.equal(result.created, 1);
  assert.equal(calls[0].command, "read_apple_inputs");
  assert.equal(calls[0].args.request.operation, "reconcile_calendar");
  assert.equal(calls[0].args.request.arguments.projections.length, 1);
});

test("permission denial and helper failure stay fail-soft", () => {
  const denied = parseApplePublicationStatus({
    source: "calendar",
    permission: "denied",
    observedAt: "2026-07-29T00:00:00Z",
    records: [],
    truncated: false,
    error: "Calendar write permission is required",
  });
  assert.equal(denied.state, "permission_required");

  const unavailable = parseApplePublicationStatus({
    source: "calendar",
    permission: "unavailable",
    observedAt: "2026-07-29T00:00:00Z",
    records: [],
    truncated: false,
    error: "helper unavailable",
  });
  assert.equal(unavailable.state, "unavailable");
});
