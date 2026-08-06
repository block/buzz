import assert from "node:assert/strict";
import test from "node:test";

import { MOCK_ARTILLERY_AGENTS } from "./mockAgents.ts";
import {
  createArtilleryChannelEnvelope,
  runArtilleryMatch,
  validateArtilleryAction,
} from "./referee.ts";

test("validates the structured agent action boundary", () => {
  assert.deepEqual(
    validateArtilleryAction({
      angle: 42.24,
      power: 70.76,
      taunt: "  incoming  ",
      weapon: "pulse-shell",
    }),
    {
      angle: 42.2,
      power: 70.8,
      taunt: "incoming",
      weapon: "pulse-shell",
    },
  );
  assert.equal(
    validateArtilleryAction({ angle: 120, power: 70, weapon: "pulse-shell" }),
    null,
  );
});

test("produces an identical authoritative transcript for identical moves", async () => {
  const first = await runArtilleryMatch({ agents: MOCK_ARTILLERY_AGENTS });
  const second = await runArtilleryMatch({ agents: MOCK_ARTILLERY_AGENTS });

  assert.deepEqual(second, first);
  assert.equal(first.winner, "red");
  assert.equal(first.turns.length, 5);
  assert.equal(first.turns[3].resolution, "invalid-fallback");
  assert.equal(first.turns.at(-1).manifest.damage.after, 0);
});

test("times out an unresponsive agent and applies the safe move", async () => {
  const match = await runArtilleryMatch({
    agents: {
      red: {
        id: "slow-red",
        name: "Slow Red",
        side: "red",
        decide: () => new Promise(() => {}),
      },
      blue: MOCK_ARTILLERY_AGENTS.blue,
    },
    maxTurns: 1,
    moveTimeoutMs: 5,
  });

  assert.equal(match.turns[0].resolution, "timeout-fallback");
  assert.equal(match.turns[0].action.power, 68);
});

test("wraps a completed match in the versioned channel event boundary", async () => {
  const match = await runArtilleryMatch({ agents: MOCK_ARTILLERY_AGENTS });
  const envelope = createArtilleryChannelEnvelope(match);

  assert.equal(envelope.type, "buzz.game.artillery.match.v1");
  assert.equal(envelope.version, 1);
  assert.equal(envelope.match.id, match.id);
});

test("streams every request and authoritative partial transcript", async () => {
  const requests = [];
  const partialTurnCounts = [];
  const match = await runArtilleryMatch({
    agents: MOCK_ARTILLERY_AGENTS,
    maxTurns: 3,
    onTurnRequest: ({ agent, state }) => {
      requests.push(`${state.turn}:${state.activeSide}:${agent.name}`);
    },
    onTurnResolved: ({ match: partialMatch }) => {
      partialTurnCounts.push(partialMatch.turns.length);
    },
  });

  assert.deepEqual(requests, ["1:red:Bumble", "2:blue:Fizz", "3:red:Bumble"]);
  assert.deepEqual(partialTurnCounts, [1, 2, 3]);
  assert.equal(match.turns.length, 3);
});

test("resumes from the last canonical turn without replaying agent decisions", async () => {
  const first = await runArtilleryMatch({
    agents: MOCK_ARTILLERY_AGENTS,
    maxTurns: 2,
  });
  const requestedTurns = [];
  const resumed = await runArtilleryMatch({
    agents: MOCK_ARTILLERY_AGENTS,
    id: first.id,
    maxTurns: 4,
    onTurnRequest: ({ state }) => requestedTurns.push(state.turn),
    resumeMatch: first,
  });

  assert.deepEqual(requestedTurns, [3, 4]);
  assert.deepEqual(resumed.turns.slice(0, 2), first.turns);
  assert.equal(resumed.turns.length, 4);
});
