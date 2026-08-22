import assert from "node:assert/strict";
import test from "node:test";

import {
  canPublishActivityLedgerTodaySnapshot,
  loadActivityLedgerTodayAuthority,
} from "./useActivityLedgerTodaySnapshot.ts";

test("Today publishing waits for a successful managed-agent roster read", () => {
  assert.equal(canPublishActivityLedgerTodaySnapshot(undefined, true), false);
  assert.equal(canPublishActivityLedgerTodaySnapshot("owner-a", false), false);
  assert.equal(canPublishActivityLedgerTodaySnapshot("owner-a", true), true);
});

test("Today authority is loaded by retained journal id across day boundaries", async () => {
  const calls = [];
  const artifacts = await loadActivityLedgerTodayAuthority(
    ["journal-across-midnight", "journal-today", "journal-across-midnight"],
    async (journalId) => {
      calls.push(journalId);
      return [
        {
          journalId,
          createdAt: journalId === "journal-across-midnight" ? 1 : 2,
        },
      ];
    },
  );

  assert.deepEqual(calls, ["journal-across-midnight", "journal-today"]);
  assert.deepEqual(
    artifacts.map((artifact) => artifact.journalId),
    ["journal-across-midnight", "journal-today"],
  );
  assert.equal(
    artifacts[0].createdAt,
    1,
    "an owner edit created before today's range must remain attached",
  );
});
