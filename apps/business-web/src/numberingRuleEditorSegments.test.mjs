import assert from "node:assert/strict";
import test from "node:test";
import {
  appendEditableSegment,
  changeEditableScope,
  createEditableSegments,
  moveEditableSegment,
  replaceEditableSegment,
} from "./numberingRuleEditorSegments.ts";

test("keeps a fixed segment identity stable while its text changes", () => {
  const rows = createEditableSegments([
    { type: "fixed", value: "SO-" },
    { type: "sequence", width: 6 },
  ]);
  const key = rows[0].key;
  const changed = replaceEditableSegment(rows, 0, {
    type: "fixed",
    value: "SALES-",
  });

  assert.equal(changed[0].key, key);
  assert.equal(changed[0].segment.value, "SALES-");
});

test("preserves identities when moving rows and creates new ones only when adding", () => {
  const rows = createEditableSegments([
    { type: "fixed", value: "PO-" },
    { type: "sequence", width: 6 },
  ]);
  const moved = moveEditableSegment(rows, 0, 1);
  const appended = appendEditableSegment(moved, {
    type: "date",
    format: "YYYYMM",
  });
  const scoped = changeEditableScope(appended, "legal_entity");

  assert.deepEqual(
    moved.map((row) => row.key),
    [rows[1].key, rows[0].key],
  );
  assert.equal(new Set(appended.map((row) => row.key)).size, 3);
  assert.equal(scoped.filter((row) => row.segment.type === "scope").length, 1);
  assert.equal(new Set(scoped.map((row) => row.key)).size, scoped.length);
});
