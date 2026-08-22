import assert from "node:assert/strict";
import test from "node:test";

import {
  activityLedgerTodaySnapshotDayGate,
  activityLedgerTodaySnapshotExpiresAt,
  canPublishActivityLedgerTodaySnapshot,
  createActivityLedgerTodayPublicationCoordinator,
  isActivityLedgerTodayArchiveFenceError,
  loadActivityLedgerTodayAuthority,
} from "./useActivityLedgerTodaySnapshot.ts";

test("Today publication retries only transient archive fence changes", () => {
  assert.equal(
    isActivityLedgerTodayArchiveFenceError(
      "Today snapshot archive revision changed: declared 7, current 8",
    ),
    true,
  );
  assert.equal(
    isActivityLedgerTodayArchiveFenceError(
      new Error("Today snapshot archive fence requires completed backfill"),
    ),
    true,
  );
  assert.equal(
    isActivityLedgerTodayArchiveFenceError("invalid owner signature"),
    false,
  );
});

test("Today publishing waits for a successful managed-agent roster read", () => {
  assert.equal(canPublishActivityLedgerTodaySnapshot(undefined, true), false);
  assert.equal(canPublishActivityLedgerTodaySnapshot("owner-a", false), false);
  assert.equal(canPublishActivityLedgerTodaySnapshot("owner-a", true), true);
});

test("Today snapshot day gate never publishes a previous-day reconstruction", () => {
  assert.deepEqual(
    activityLedgerTodaySnapshotDayGate(
      "2026-08-21",
      new Date("2026-08-22T00:00:01"),
      false,
    ),
    { action: "rebuild", day: "2026-08-22" },
  );
  assert.deepEqual(
    activityLedgerTodaySnapshotDayGate(
      "2026-08-21",
      new Date("2026-08-22T00:00:01"),
      true,
    ),
    { action: "discard", day: "2026-08-22" },
  );
});

test("Today snapshot validity is clipped at the next local midnight", () => {
  const midday = new Date(2026, 7, 21, 12, 0, 0);
  assert.equal(
    activityLedgerTodaySnapshotExpiresAt(midday),
    Math.floor(midday.getTime() / 1_000) + 5 * 60,
  );

  const beforeMidnight = new Date(2026, 7, 21, 23, 59, 30);
  const midnight = new Date(2026, 7, 22, 0, 0, 0);
  assert.equal(
    activityLedgerTodaySnapshotExpiresAt(beforeMidnight),
    Math.floor(midnight.getTime() / 1_000),
  );
});

test("a retired hung roster generation does not block its replacement", async () => {
  const coordinator = createActivityLedgerTodayPublicationCoordinator();
  const writes = [];
  const hungBuild = new Promise(() => {});
  const oldGeneration = coordinator.beginGeneration();
  void hungBuild.then(() => {
    if (coordinator.isCurrent(oldGeneration)) writes.push("old-roster");
  });
  coordinator.invalidate(oldGeneration);
  const newGeneration = coordinator.beginGeneration();
  assert.equal(coordinator.isCurrent(newGeneration), true);
  await Promise.resolve().then(() => writes.push("new-roster"));
  assert.deepEqual(writes, ["new-roster"]);
});

test("a late older write requests a current-roster repair", () => {
  const coordinator = createActivityLedgerTodayPublicationCoordinator();
  const writes = [];
  const oldGeneration = coordinator.beginGeneration();
  coordinator.invalidate(oldGeneration);
  const newGeneration = coordinator.beginGeneration();
  coordinator.setCurrentRepublish(newGeneration, () => writes.push("repair"));
  writes.push("new-finished", "old-finished");
  coordinator.noteWriteCompleted(oldGeneration);
  assert.deepEqual(writes, ["new-finished", "old-finished", "repair"]);
});

test("Today authority is loaded by retained journal id across day boundaries", async () => {
  const calls = [];
  const artifacts = await loadActivityLedgerTodayAuthority(
    "wss://relay.example",
    [
      { agentPubkey: "agent-a", journalId: "journal-across-midnight" },
      { agentPubkey: "agent-b", journalId: "journal-today" },
      { agentPubkey: "agent-a", journalId: "journal-across-midnight" },
    ],
    async (relayUrl, agentPubkey, journalId) => {
      calls.push([relayUrl, agentPubkey, journalId]);
      return [
        {
          agentPubkey,
          journalId,
          createdAt: journalId === "journal-across-midnight" ? 1 : 2,
        },
      ];
    },
  );

  assert.deepEqual(calls, [
    ["wss://relay.example", "agent-a", "journal-across-midnight"],
    ["wss://relay.example", "agent-b", "journal-today"],
  ]);
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

test("Today authority keeps same journal id isolated across agents", async () => {
  const calls = [];
  await loadActivityLedgerTodayAuthority(
    "wss://relay.example",
    [
      { agentPubkey: "agent-a", journalId: "shared-turn" },
      { agentPubkey: "agent-b", journalId: "shared-turn" },
    ],
    async (_relayUrl, agentPubkey, journalId) => {
      calls.push([agentPubkey, journalId]);
      return [];
    },
  );
  assert.deepEqual(calls, [
    ["agent-a", "shared-turn"],
    ["agent-b", "shared-turn"],
  ]);
});
