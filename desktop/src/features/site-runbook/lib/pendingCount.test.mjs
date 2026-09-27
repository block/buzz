import assert from "node:assert/strict";
import test from "node:test";

import {
  countPendingInRunbook,
  countPendingProcedures,
} from "./pendingCount.ts";
import { emptyRunbook, emptyRunbooksBlob } from "./serialize.ts";

function proc(id, status) {
  return {
    id,
    title: id,
    steps: "s",
    status,
    createdAt: 1,
    updatedAt: 1,
  };
}

test("countPendingInRunbook counts only pending", () => {
  assert.equal(countPendingInRunbook(null), 0);
  assert.equal(countPendingInRunbook(emptyRunbook()), 0);
  const runbook = {
    ...emptyRunbook(1),
    procedures: [
      proc("a", "pending"),
      proc("b", "active"),
      proc("c", "pending"),
      proc("d", "archived"),
    ],
  };
  assert.equal(countPendingInRunbook(runbook), 2);
});

test("countPendingProcedures sums across blob runbooks", () => {
  assert.equal(countPendingProcedures(null), 0);
  assert.equal(countPendingProcedures(emptyRunbooksBlob()), 0);
  const blob = {
    version: 1,
    runbooks: {
      "sid:one": {
        ...emptyRunbook(1),
        procedures: [proc("p1", "pending"), proc("p2", "active")],
      },
      "pin:two": {
        ...emptyRunbook(2),
        procedures: [proc("p3", "pending"), proc("p4", "pending")],
      },
    },
  };
  assert.equal(countPendingProcedures(blob), 3);
});
