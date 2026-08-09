import assert from "node:assert/strict";
import test from "node:test";

const { buildRollbackPreview } = await import("./rollback.ts");

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

test("rollback is a new reviewed inverse revision, not a rewind", () => {
  const one = event("one", "r1");
  const two = event("two", "r1");
  const twoChanged = event("two", "r2", "Two changed");
  const revisions = [
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
      changes: [
        { kind: "removed", before: one },
        { kind: "changed", before: two, after: twoChanged },
      ],
    },
  ];

  const preview = buildRollbackPreview({
    ownerPubkey: "owner",
    source: {
      schemaVersion: 1,
      id: "fas",
      type: "fas",
      displayName: "FAS",
      coverageStart: "2026-08-01T00:00:00+10:00",
      coverageEnd: "2026-09-01T00:00:00+10:00",
      documentName: "FAS.xlsx",
      documentHash: "a".repeat(64),
      revisionId: "r2",
      priorRevisionId: "r1",
      importedAt: "2026-07-02T00:00:00Z",
      status: "approved",
      sourceReference: "local",
    },
    revisions,
    targetRevisionId: "r1",
    revisionId: "rollback-r3",
    importedAt: "2026-07-03T00:00:00Z",
  });

  assert.equal(preview.added, 1);
  assert.equal(preview.changed, 1);
  assert.equal(preview.removed, 0);
  assert.equal(preview.input.revision.priorRevisionId, "r2");
  assert.equal(preview.input.source.revisionId, "rollback-r3");
  assert.equal(preview.input.events.length, 2);
  assert.ok(
    preview.input.events.every(
      (item) => item.ownership.revisionId === "rollback-r3",
    ),
  );
});
