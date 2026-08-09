import assert from "node:assert/strict";
import test from "node:test";

const { buildImportRevision, interpretExtractedDocument, parseImportProposal } =
  await import("./importDiff.ts");

const coverage = {
  start: "2026-07-27T00:00:00+10:00",
  end: "2026-08-03T00:00:00+10:00",
};

test("strict proposal parser rejects prose, missing evidence, and out-of-coverage events", () => {
  const valid = {
    schemaVersion: 1,
    sourceType: "shortcast",
    proposedCoverage: coverage,
    events: [
      {
        title: "Navigation brief",
        type: "brief",
        start: "2026-07-29T08:00:00+10:00",
        end: "2026-07-29T09:00:00+10:00",
        allDay: false,
        location: null,
        responsibleOwner: "Navigator",
        participants: [],
        remarks: null,
        sourceLocation: "Shortcast!A2:C2",
      },
    ],
    uncertainties: [],
  };
  assert.equal(parseImportProposal(valid).events.length, 1);
  assert.throws(
    () => parseImportProposal(`Here is the JSON: ${JSON.stringify(valid)}`),
    /Import proposal/,
  );
  assert.throws(
    () =>
      parseImportProposal({
        ...valid,
        events: [{ ...valid.events[0], sourceLocation: "" }],
      }),
    /Import proposal/,
  );
  assert.throws(
    () =>
      parseImportProposal({
        ...valid,
        events: [
          {
            ...valid.events[0],
            start: "2027-01-01T08:00:00+11:00",
            end: "2027-01-01T09:00:00+11:00",
          },
        ],
      }),
    /coverage/,
  );
});

test("deterministic Shortcast interpretation retains row evidence", () => {
  const proposal = interpretExtractedDocument(
    {
      filename: "Shortcast.docx",
      extension: "docx",
      sha256: "a".repeat(64),
      sizeBytes: 100,
      blocks: [
        {
          kind: "table_row",
          location: "table 1 row 1",
          cells: ["Date", "Time", "Event"],
        },
        {
          kind: "table_row",
          location: "table 1 row 2",
          cells: ["29 Jul 2026", "0800", "Navigation brief"],
        },
      ],
      pages: [],
      sheets: [],
      truncated: false,
    },
    "shortcast",
    coverage,
    "Australia/Sydney",
  );

  assert.equal(proposal.events.length, 1);
  assert.equal(proposal.events[0].title, "Navigation brief");
  assert.equal(proposal.events[0].start, "2026-07-29T08:00:00+10:00");
  assert.equal(proposal.events[0].sourceLocation, "table 1 row 2");
});

test("diff changes source-owned matches, removes only inside coverage, and preserves manual and other sources", () => {
  const existing = [
    {
      schemaVersion: 1,
      id: "source-a:brief",
      ownership: {
        kind: "source",
        sourceId: "source-a",
        revisionId: "old",
        sourceLocation: "table 1 row 2",
      },
      title: "Navigation brief",
      description: null,
      type: "brief",
      start: "2026-07-29T07:30:00+10:00",
      end: "2026-07-29T08:00:00+10:00",
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
    },
    {
      schemaVersion: 1,
      id: "source-a:removed",
      ownership: {
        kind: "source",
        sourceId: "source-a",
        revisionId: "old",
        sourceLocation: "table 1 row 3",
      },
      title: "Old event",
      description: null,
      type: "routine",
      start: "2026-07-30T08:00:00+10:00",
      end: "2026-07-30T09:00:00+10:00",
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
    },
    {
      schemaVersion: 1,
      id: "manual",
      ownership: { kind: "manual" },
      title: "CO commitment",
      description: null,
      type: "routine",
      start: "2026-07-30T08:00:00+10:00",
      end: "2026-07-30T09:00:00+10:00",
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
    },
  ];
  const proposal = parseImportProposal({
    schemaVersion: 1,
    sourceType: "shortcast",
    proposedCoverage: coverage,
    events: [
      {
        title: "Navigation brief",
        type: "brief",
        start: "2026-07-29T08:00:00+10:00",
        end: "2026-07-29T09:00:00+10:00",
        allDay: false,
        location: null,
        responsibleOwner: "Navigator",
        participants: [],
        remarks: null,
        sourceLocation: "table 1 row 2",
      },
    ],
    uncertainties: [],
  });

  const result = buildImportRevision({
    sourceId: "source-a",
    revisionId: "new",
    priorRevisionId: "old",
    importedAt: "2026-07-29T00:00:00Z",
    timeZone: "Australia/Sydney",
    proposal,
    existing,
  });

  assert.deepEqual(
    result.revision.changes.map((change) => change.kind),
    ["changed", "removed"],
  );
  assert.equal(result.events.length, 1);
  assert.equal(result.events[0].id, "source-a:brief");
  assert.equal(result.diff.preserved, 1);
});
