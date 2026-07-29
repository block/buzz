import assert from "node:assert/strict";
import test from "node:test";

import { drainPendingAgentSnapshotImport } from "./agentSnapshotHandoff.ts";

test("handoff drain routes accepted bytes through the existing preview flow", async () => {
  const calls = [];
  const pending = {
    id: "550e8400-e29b-41d4-a716-446655440000",
    fileBytes: [1, 2, 3],
    fileName: "550e8400-e29b-41d4-a716-446655440000.agent.json",
    snapshotKind: "agent",
  };
  let routedPayload;

  const accepted = await drainPendingAgentSnapshotImport({
    take: async () => pending,
    acknowledge: async (id) => {
      calls.push(["ack", id]);
      return true;
    },
    requestOpen: (payload) => {
      routedPayload = payload;
      calls.push([
        "open",
        {
          fileBytes: payload.fileBytes,
          fileName: payload.fileName,
          snapshotKind: payload.snapshotKind,
        },
      ]);
    },
    goAgents: () => calls.push(["navigate"]),
  });

  assert.equal(accepted, true);
  assert.deepEqual(calls, [
    [
      "open",
      {
        fileBytes: [1, 2, 3],
        fileName: pending.fileName,
        snapshotKind: "agent",
      },
    ],
    ["navigate"],
  ]);
  assert.equal(typeof routedPayload.onPreviewAccepted, "function");
  await routedPayload.onPreviewAccepted();
  assert.deepEqual(calls.at(-1), ["ack", pending.id]);
});

test("handoff drain does nothing when no import is pending", async () => {
  let touched = false;
  const accepted = await drainPendingAgentSnapshotImport({
    take: async () => null,
    acknowledge: async () => {
      touched = true;
      return true;
    },
    requestOpen: () => {
      touched = true;
    },
    goAgents: () => {
      touched = true;
    },
  });

  assert.equal(accepted, false);
  assert.equal(touched, false);
});

test("handoff drain does not acknowledge when preview routing throws", async () => {
  let acknowledged = false;
  await assert.rejects(() =>
    drainPendingAgentSnapshotImport({
      take: async () => ({
        id: "550e8400-e29b-41d4-a716-446655440000",
        fileBytes: [1],
        fileName: "snapshot.agent.json",
        snapshotKind: "agent",
      }),
      acknowledge: async () => {
        acknowledged = true;
        return true;
      },
      requestOpen: () => {
        throw new Error("route failed");
      },
      goAgents: () => {},
    }),
  );
  assert.equal(acknowledged, false);
});
