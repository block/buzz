import assert from "node:assert/strict";
import test from "node:test";
import {
  interpretPlanDocument,
  parsePlanImportProposal,
} from "./importProposal.ts";
import { planningProject } from "./testFixtures.ts";

function document(rows) {
  return {
    filename: "NT Planning.xlsx",
    extension: "xlsx",
    sha256: "a".repeat(64),
    sizeBytes: 4096,
    blocks: rows.flatMap((values, rowIndex) =>
      values.map((value, columnIndex) => ({
        kind: "spreadsheet_cell",
        location: `Planning!${String.fromCharCode(65 + columnIndex)}${rowIndex + 1}`,
        sheet: "Planning",
        coordinate: `${String.fromCharCode(65 + columnIndex)}${rowIndex + 1}`,
        value,
      })),
    ),
    pages: [],
    sheets: [{ name: "Planning", maximumRow: rows.length, maximumColumn: 8 }],
    truncated: false,
  };
}

test("interprets an NT-style WBS table with exact source evidence", () => {
  const proposal = interpretPlanDocument(
    document([
      [
        "WBS",
        "Task",
        "Owner",
        "Start",
        "Due",
        "Duration",
        "Progress",
        "Dependencies",
      ],
      [
        "1",
        "Define support concept",
        "Operations Officer",
        "2026-08-03",
        "2026-08-04",
        "2",
        "100%",
        "",
      ],
      [
        "2",
        "Confirm logistics support",
        "Logistics Officer",
        "2026-08-05",
        "2026-08-07",
        "3",
        "25%",
        "1",
      ],
    ]),
    planningProject,
  );

  assert.equal(proposal.project.id, planningProject.id);
  assert.deepEqual(proposal.tasks, [
    {
      wbs: "1",
      title: "Define support concept",
      owner: "Operations Officer",
      plannedStart: "2026-08-03",
      dueDate: "2026-08-04",
      durationWorkdays: 2,
      percentComplete: 100,
      dependencyWbs: [],
      sourceLocation: "Planning!row 2",
    },
    {
      wbs: "2",
      title: "Confirm logistics support",
      owner: "Logistics Officer",
      plannedStart: "2026-08-05",
      dueDate: "2026-08-07",
      durationWorkdays: 3,
      percentComplete: 25,
      dependencyWbs: ["1"],
      sourceLocation: "Planning!row 3",
    },
  ]);
  assert.deepEqual(proposal.uncertainties, []);
});

test("surfaces unknown dependencies and duplicate WBS rows without inventing links", () => {
  const proposal = interpretPlanDocument(
    document([
      ["WBS", "Task", "Owner", "Start", "Due", "Duration", "Dependencies"],
      ["1", "First task", "Navigator", "2026-08-03", "2026-08-03", "1", ""],
      [
        "2",
        "Dependent task",
        "Operations Officer",
        "2026-08-04",
        "2026-08-04",
        "1",
        "9",
      ],
      [
        "2",
        "Duplicate task",
        "Executive Officer",
        "2026-08-05",
        "2026-08-05",
        "1",
        "",
      ],
    ]),
    planningProject,
  );

  assert.deepEqual(proposal.tasks[1].dependencyWbs, []);
  assert.equal(proposal.uncertainties.length, 2);
  assert.equal(
    proposal.uncertainties.some((item) => /dependency 9/i.test(item.message)),
    true,
  );
  assert.equal(
    proposal.uncertainties.some((item) =>
      /duplicate WBS 2/i.test(item.message),
    ),
    true,
  );
  assert.equal(
    proposal.uncertainties.every((item) => item.blocking),
    true,
  );
});

test("strict parsing rejects missing dependency references and source-less rows", () => {
  const proposal = interpretPlanDocument(
    document([
      ["WBS", "Task", "Owner", "Start", "Due", "Duration"],
      ["1", "First task", "Navigator", "2026-08-03", "2026-08-03", "1"],
    ]),
    planningProject,
  );

  assert.throws(() =>
    parsePlanImportProposal({
      ...proposal,
      tasks: [{ ...proposal.tasks[0], sourceLocation: "" }],
    }),
  );
  assert.throws(() =>
    parsePlanImportProposal({
      ...proposal,
      tasks: [{ ...proposal.tasks[0], dependencyWbs: ["missing"] }],
    }),
  );
});
