import assert from "node:assert/strict";
import test from "node:test";

const { reconstructSourceRevision } = await import("./revisionState.ts");

function event(id, revisionId, title = id) {
  return {
    schemaVersion: 1,
    id,
    ownership: {
      kind: "source",
      sourceId: "fas",
      revisionId,
      sourceLocation: `row:${id}`,
    },
    title,
    description: null,
    type: "activity",
    start: "2026-08-03T08:00:00+10:00",
    end: "2026-08-03T09:00:00+10:00",
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
}

test("reconstructs removals through later unrelated revisions", () => {
  const one = event("one", "r1");
  const two = event("two", "r1");
  const twoChanged = event("two", "r3", "Two updated");
  const state = reconstructSourceRevision(
    [
      {
        schemaVersion: 1,
        id: "r1",
        sourceId: "fas",
        priorRevisionId: null,
        importedAt: "2026-07-01T00:00:00Z",
        changes: [
          { kind: "added", after: one },
          { kind: "added", after: two },
        ],
      },
      {
        schemaVersion: 1,
        id: "r2",
        sourceId: "fas",
        priorRevisionId: "r1",
        importedAt: "2026-07-02T00:00:00Z",
        changes: [{ kind: "removed", before: one }],
      },
      {
        schemaVersion: 1,
        id: "r3",
        sourceId: "fas",
        priorRevisionId: "r2",
        importedAt: "2026-07-03T00:00:00Z",
        changes: [{ kind: "changed", before: two, after: twoChanged }],
      },
    ],
    "fas",
    "r3",
  );

  assert.deepEqual([...state.keys()], ["two"]);
  assert.equal(state.get("two").title, "Two updated");
});

test("rejects an incomplete, cyclic, or contradictory chain", () => {
  const one = event("one", "r1");
  assert.throws(() =>
    reconstructSourceRevision(
      [
        {
          schemaVersion: 1,
          id: "r2",
          sourceId: "fas",
          priorRevisionId: "missing",
          importedAt: "2026-07-02T00:00:00Z",
          changes: [{ kind: "removed", before: one }],
        },
      ],
      "fas",
      "r2",
    ),
  );
});
