import assert from "node:assert/strict";
import test from "node:test";

import { expandRecurringEvents } from "./occurrences.ts";

const weekly = {
  schemaVersion: 1,
  id: "cub",
  ownership: { kind: "manual" },
  title: "Commanders Update Brief",
  description: null,
  type: "meeting",
  start: "2026-07-29T08:00:00+10:00",
  end: "2026-07-29T09:00:00+10:00",
  allDay: false,
  timeZone: "Australia/Sydney",
  status: "approved",
  location: null,
  responsibleOwner: null,
  participants: [],
  remarks: null,
  linkedPlanId: null,
  linkedTaskId: null,
  linkedMissionRequirementId: null,
  parentActivityId: null,
  recurrence: {
    frequency: "weekly",
    interval: 1,
    until: "2026-08-12T08:00:00+10:00",
    seriesId: "cub-series",
  },
  excludedOccurrenceStarts: ["2026-08-05T08:00:00+10:00"],
};

test("weekly expansion omits exclusions and retains later occurrences", () => {
  assert.deepEqual(
    expandRecurringEvents([weekly], {
      start: "2026-07-27T00:00:00+10:00",
      end: "2026-08-17T00:00:00+10:00",
    }).map((event) => event.start),
    ["2026-07-29T08:00:00+10:00", "2026-08-12T08:00:00+10:00"],
  );
});
