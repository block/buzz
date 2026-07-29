import assert from "node:assert/strict";
import test from "node:test";
import { applyLocalAdjustments } from "./localAdjustments.ts";

const base = {
  schemaVersion: 1,
  id: "source-event",
  ownership: {
    kind: "source",
    sourceId: "fas",
    revisionId: "revision-1",
    sourceLocation: "row 4",
  },
  title: "Sail",
  description: null,
  type: "activity",
  start: "2026-08-10T08:00:00+10:00",
  end: "2026-08-10T09:00:00+10:00",
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
  recurrence: null,
  excludedOccurrenceStarts: [],
};

test("a local adjustment overlays but does not mutate its source event", () => {
  const adjustment = {
    ...base,
    id: "adjustment",
    ownership: { kind: "manual" },
    start: "2026-08-11T08:00:00+10:00",
    end: "2026-08-11T09:00:00+10:00",
    parentActivityId: base.id,
  };
  assert.deepEqual(applyLocalAdjustments([base, adjustment]), [adjustment]);
  assert.equal(base.start, "2026-08-10T08:00:00+10:00");
});
