import assert from "node:assert/strict";
import test from "node:test";
import { evaluatePlanningChecks } from "./deterministicChecks.ts";

function event(overrides = {}) {
  return {
    schemaVersion: 1,
    id: "sailing",
    ownership: { kind: "manual" },
    title: "Sail Manila",
    description: null,
    type: "activity",
    start: "2026-08-03T08:00:00+10:00",
    end: "2026-08-03T10:00:00+10:00",
    allDay: false,
    timeZone: "Australia/Sydney",
    status: "approved",
    location: "Manila",
    responsibleOwner: "Operations Officer",
    participants: [],
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

test("flags Friday securing-for-sea rounds missing before a Monday sailing", () => {
  const findings = evaluatePlanningChecks({
    events: [event()],
    sources: [],
    timeZone: "Australia/Sydney",
  });

  assert.equal(findings.length, 1);
  assert.equal(findings[0].category, "missingPrerequisite");
  assert.equal(findings[0].affectedEventIds[0], "sailing");
  assert.equal(findings[0].proposedEvent.start, "2026-07-31T00:00:00+10:00");
  assert.equal(findings[0].proposedEvent.title, "Securing for sea rounds");
});

test("does not warn when the prerequisite exists or sailing is cancelled", () => {
  const securing = event({
    id: "rounds",
    title: "Securing for sea rounds",
    start: "2026-07-31T14:00:00+10:00",
    end: "2026-07-31T16:00:00+10:00",
  });
  assert.deepEqual(
    evaluatePlanningChecks({
      events: [event(), securing],
      sources: [],
      timeZone: "Australia/Sydney",
    }),
    [],
  );
  assert.deepEqual(
    evaluatePlanningChecks({
      events: [event({ status: "cancelled" })],
      sources: [],
      timeZone: "Australia/Sydney",
    }),
    [],
  );
});

test("flags a bounded FAS and Shortcast date conflict for the same activity", () => {
  const sources = [
    { id: "fas", type: "fas" },
    { id: "shortcast", type: "shortcast" },
  ];
  const findings = evaluatePlanningChecks({
    events: [
      event({
        id: "fas-sail",
        ownership: {
          kind: "source",
          sourceId: "fas",
          revisionId: "fas-r1",
          sourceLocation: "FAS row 5",
        },
        start: "2026-08-04T08:00:00+10:00",
        end: "2026-08-04T10:00:00+10:00",
      }),
      event({
        id: "shortcast-sail",
        ownership: {
          kind: "source",
          sourceId: "shortcast",
          revisionId: "shortcast-r1",
          sourceLocation: "Shortcast row 2",
        },
      }),
    ],
    sources,
    timeZone: "Australia/Sydney",
  });

  assert.equal(
    findings.some((finding) => finding.category === "sourceConflict"),
    true,
  );
  const conflict = findings.find(
    (finding) => finding.category === "sourceConflict",
  );
  assert.deepEqual(conflict?.affectedEventIds, ["fas-sail", "shortcast-sail"]);
  assert.match(conflict?.rationale ?? "", /Monday/i);
  assert.match(conflict?.rationale ?? "", /Tuesday/i);
});
