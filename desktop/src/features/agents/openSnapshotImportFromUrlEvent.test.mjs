import assert from "node:assert/strict";
import test from "node:test";

import {
  acceptPendingSnapshotImport,
  claimPendingSnapshotImport,
  consumePendingSnapshotImport,
  rejectPendingSnapshotImport,
  releasePendingSnapshotImport,
  requestOpenSnapshotImport,
} from "./openSnapshotImportFromUrlEvent.ts";

function reset() {
  while (true) {
    const pending = consumePendingSnapshotImport();
    if (!pending) return;
    claimPendingSnapshotImport(pending.id);
    acceptPendingSnapshotImport(pending.id);
    assert.equal(releasePendingSnapshotImport(pending.id), true);
  }
}

function team(id) {
  return {
    id,
    fileBytes: [id.length],
    fileName: `${id}.buzzteam`,
    snapshotKind: "team",
  };
}

test("openSnapshotImportFromUrlEvent: no request produces no pending import", () => {
  reset();
  assert.equal(consumePendingSnapshotImport(), null);
});

test("openSnapshotImportFromUrlEvent: acceptance retains route ownership until state-clearing release", () => {
  reset();
  requestOpenSnapshotImport(team("first"));

  assert.equal(consumePendingSnapshotImport()?.id, "first");
  assert.equal(claimPendingSnapshotImport("first"), true);
  assert.equal(acceptPendingSnapshotImport("first"), true);
  assert.equal(
    consumePendingSnapshotImport()?.id,
    "first",
    "acceptance, result, or confirm error must not release the dialog owner",
  );
});

test("openSnapshotImportFromUrlEvent: serializes two distinct ids until the first dialog closes", () => {
  reset();
  requestOpenSnapshotImport(team("first"));
  requestOpenSnapshotImport(team("second"));

  assert.equal(consumePendingSnapshotImport()?.id, "first");
  assert.equal(claimPendingSnapshotImport("first"), true);
  assert.equal(acceptPendingSnapshotImport("first"), true);
  assert.equal(
    consumePendingSnapshotImport()?.id,
    "first",
    "item two must stay withheld while item one owns preview or result state",
  );

  assert.equal(releasePendingSnapshotImport("first"), true);
  assert.equal(consumePendingSnapshotImport()?.id, "second");
  assert.equal(claimPendingSnapshotImport("second"), true);
  assert.equal(acceptPendingSnapshotImport("second"), true);
  assert.equal(releasePendingSnapshotImport("second"), true);
  assert.equal(consumePendingSnapshotImport(), null);
});

test("openSnapshotImportFromUrlEvent: failed confirmation keeps the first request until its later dialog close", () => {
  reset();
  requestOpenSnapshotImport(team("first"));
  requestOpenSnapshotImport(team("second"));

  assert.equal(claimPendingSnapshotImport("first"), true);
  assert.equal(acceptPendingSnapshotImport("first"), true);
  // Failed confirmation and a successful retry result both leave dialog state owned.
  assert.equal(consumePendingSnapshotImport()?.id, "first");
  assert.equal(releasePendingSnapshotImport("first"), true);
  assert.equal(consumePendingSnapshotImport()?.id, "second");
});

test("openSnapshotImportFromUrlEvent: preview rejection acknowledges the surfaced failure and unblocks the next request", () => {
  reset();
  const rejected = [];
  requestOpenSnapshotImport({
    ...team("first"),
    onRejected: (id) => rejected.push(id),
  });
  requestOpenSnapshotImport(team("second"));

  assert.equal(rejectPendingSnapshotImport("first"), true);
  assert.deepEqual(rejected, ["first"]);
  assert.equal(consumePendingSnapshotImport()?.id, "second");
});

test("openSnapshotImportFromUrlEvent: unknown and non-head ids cannot advance the queue", () => {
  reset();
  requestOpenSnapshotImport(team("first"));
  requestOpenSnapshotImport(team("second"));

  assert.equal(acceptPendingSnapshotImport("unknown"), false);
  assert.equal(releasePendingSnapshotImport("second"), false);
  assert.equal(consumePendingSnapshotImport()?.id, "first");
});
