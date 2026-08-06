import assert from "node:assert/strict";
import test from "node:test";

import { liveArtilleryMatchController } from "./liveMatchController.ts";

const MOVE = { angle: 45, power: 72, weapon: "pulse-shell" };

test.afterEach(() => {
  liveArtilleryMatchController.reset();
});

test("persists and streams a two-agent match outside React route state", async () => {
  const turnCounts = [];
  const statuses = [];
  const unsubscribe = liveArtilleryMatchController.subscribe(() => {
    const snapshot = liveArtilleryMatchController.getSnapshot();
    statuses.push(snapshot.status);
    if (snapshot.match) turnCounts.push(snapshot.match.turns.length);
  });

  const matchPromise = liveArtilleryMatchController.start({
    agents: {
      red: {
        id: "red-agent",
        name: "Red Agent",
        side: "red",
        decide: async () => MOVE,
      },
      blue: {
        id: "blue-agent",
        name: "Blue Agent",
        side: "blue",
        decide: async () => ({ ...MOVE, power: 75 }),
      },
    },
    channelId: "game-channel",
    id: "live-controller-test",
    maxTurns: 4,
    statusEventId: "status-event",
    timeoutMs: 50,
  });
  const match = await matchPromise;
  unsubscribe();
  const snapshot = liveArtilleryMatchController.getSnapshot();
  assert.equal(match.turns.length, 4);
  assert.equal(snapshot.status, "complete");
  assert.equal(snapshot.matchComplete, true);
  assert.equal(snapshot.channelId, "game-channel");
  assert.equal(snapshot.statusEventId, "status-event");
  assert.equal(snapshot.match.turns.length, 4);
  assert.ok(statuses.includes("waiting"));
  assert.ok(statuses.includes("running"));
  assert.deepEqual([...new Set(turnCounts)].sort(), [0, 1, 2, 3, 4]);
});

test("exposes the waiting agent and completes with a timeout fallback", async () => {
  const matchPromise = liveArtilleryMatchController.start({
    agents: {
      red: {
        id: "quiet-red",
        name: "Quiet Red",
        side: "red",
        decide: () => new Promise(() => {}),
      },
      blue: {
        id: "blue-agent",
        name: "Blue Agent",
        side: "blue",
        decide: async () => MOVE,
      },
    },
    channelId: "game-channel",
    maxTurns: 1,
    timeoutMs: 5,
  });

  const waiting = liveArtilleryMatchController.getSnapshot();
  assert.equal(waiting.waitingFor.agentName, "Quiet Red");
  assert.equal(waiting.waitingFor.turn, 1);
  assert.equal(waiting.waitingFor.side, "red");

  const match = await matchPromise;
  assert.equal(match.turns[0].resolution, "timeout-fallback");
  assert.equal(liveArtilleryMatchController.getSnapshot().status, "complete");
});
