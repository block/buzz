import assert from "node:assert/strict";
import test from "node:test";
import { deriveShipRoutinePeriods, shipStateAt } from "./shipRoutine.ts";

const source = {
  schemaVersion: 1,
  id: "fas-1",
  type: "fas",
  displayName: "Fleet Activity Schedule",
  coverageStart: "2026-08-01T00:00:00+10:00",
  coverageEnd: "2026-09-01T00:00:00+10:00",
  documentName: "FAS.xlsx",
  documentHash: "a".repeat(64),
  revisionId: "rev-1",
  priorRevisionId: null,
  importedAt: "2026-07-29T00:00:00Z",
  status: "approved",
  sourceReference: "FAS August",
};

function event(id, type, start, end, remarks = null) {
  return {
    schemaVersion: 1,
    id,
    ownership: {
      kind: "source",
      sourceId: "fas-1",
      revisionId: "rev-1",
      sourceLocation: id,
    },
    title: id,
    description: null,
    type,
    start,
    end,
    allDay: false,
    timeZone: "Australia/Sydney",
    status: "approved",
    location: null,
    responsibleOwner: null,
    participants: [],
    remarks,
    linkedPlanId: null,
    linkedTaskId: null,
    linkedMissionRequirementId: null,
    parentActivityId: null,
    recurrence: null,
    excludedOccurrenceStarts: [],
  };
}

test("derives FAS alongside and at-sea periods and Shortcast Ship Time", () => {
  const periods = deriveShipRoutinePeriods(
    [source],
    [
      event(
        "Sydney",
        "routine_alongside",
        "2026-08-01T00:00:00+10:00",
        "2026-08-05T08:00:00+10:00",
      ),
      event(
        "Sea",
        "routine_at_sea",
        "2026-08-05T08:00:00+10:00",
        "2026-08-12T08:00:00+10:00",
      ),
      event(
        "Zone",
        "timezone_change",
        "2026-08-08T02:00:00+10:00",
        "2026-08-08T02:01:00+10:00",
        "Asia/Manila",
      ),
    ],
    {
      start: "2026-08-01T00:00:00+10:00",
      end: "2026-08-15T00:00:00+10:00",
    },
  );

  assert.equal(
    shipStateAt(periods, "2026-08-03T12:00:00+10:00").routine,
    "alongside",
  );
  assert.equal(
    shipStateAt(periods, "2026-08-06T12:00:00+10:00").routine,
    "atSea",
  );
  assert.equal(
    shipStateAt(periods, "2026-08-09T12:00:00+10:00").timeZone,
    "Asia/Manila",
  );
});

test("carries the last routine through uncovered time and reports invalid zones", () => {
  const periods = deriveShipRoutinePeriods(
    [source],
    [
      event(
        "Sea",
        "routine_at_sea",
        "2026-08-01T00:00:00+10:00",
        "2026-08-02T00:00:00+10:00",
      ),
      event(
        "Bad zone",
        "timezone_change",
        "2026-08-03T00:00:00+10:00",
        "2026-08-03T00:01:00+10:00",
        "Mars/Olympus",
      ),
    ],
    {
      start: "2026-08-01T00:00:00+10:00",
      end: "2026-08-05T00:00:00+10:00",
    },
  );

  const carried = shipStateAt(periods, "2026-08-04T12:00:00+10:00");
  assert.equal(carried.routine, "atSea");
  assert.equal(carried.assumed, true);
  assert.ok(carried.findings.join(" ").includes("Mars/Olympus"));
});
