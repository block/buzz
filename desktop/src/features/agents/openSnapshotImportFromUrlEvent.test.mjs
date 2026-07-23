import assert from "node:assert/strict";
import test from "node:test";

import {
  consumePendingSnapshotImport,
  requestOpenSnapshotImport,
} from "./openSnapshotImportFromUrlEvent.ts";

test("openSnapshotImportFromUrlEvent: consume returns null with no pending import", () => {
  // Reset state from any prior test run.
  consumePendingSnapshotImport();
  assert.equal(consumePendingSnapshotImport(), null);
});

test("openSnapshotImportFromUrlEvent: request then consume returns payload", () => {
  consumePendingSnapshotImport(); // clear
  requestOpenSnapshotImport({ fileBytes: [1, 2, 3], fileName: "x.agent.json" });
  const payload = consumePendingSnapshotImport();
  assert.ok(payload !== null);
  assert.deepEqual(payload.fileBytes, [1, 2, 3]);
  assert.equal(payload.fileName, "x.agent.json");
});

test("openSnapshotImportFromUrlEvent: consume is one-shot and clears pending", () => {
  consumePendingSnapshotImport(); // clear
  requestOpenSnapshotImport({ fileBytes: [9], fileName: "y.agent.json" });
  const first = consumePendingSnapshotImport();
  const second = consumePendingSnapshotImport();
  assert.ok(first !== null, "first consume must return payload");
  assert.equal(second, null, "second consume must return null");
});

test("openSnapshotImportFromUrlEvent: double-request replaces pending payload", () => {
  consumePendingSnapshotImport(); // clear
  requestOpenSnapshotImport({ fileBytes: [1], fileName: "a.agent.json" });
  requestOpenSnapshotImport({ fileBytes: [2], fileName: "b.agent.json" });
  const payload = consumePendingSnapshotImport();
  assert.ok(payload !== null);
  // Only the latest request survives — prevents stacking.
  assert.deepEqual(payload.fileBytes, [2]);
  assert.equal(payload.fileName, "b.agent.json");
});
