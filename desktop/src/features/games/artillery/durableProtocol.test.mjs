import assert from "node:assert/strict";
import test from "node:test";

import {
  appendArtilleryDurableEvent,
  createArtilleryFinishedEvent,
  createArtilleryStartedEvent,
  createArtilleryTurnRequestedEvent,
  createArtilleryTurnResolvedEvent,
  parseArtilleryDurableEvent,
  recoverArtilleryMatch,
  stripArtilleryDurableEvent,
} from "./durableProtocol.ts";
import { resolveArtilleryTurn } from "./referee.ts";

const agents = {
  red: { id: "red-pubkey", name: "Red Agent" },
  blue: { id: "blue-pubkey", name: "Blue Agent" },
};

function state(turn, health) {
  return {
    id: "durable-match",
    turn,
    activeSide: turn % 2 === 1 ? "red" : "blue",
    health: { ...health },
    wind: turn === 1 ? 0 : -2,
  };
}

test("round-trips a durable event without exposing its marker as plain text", () => {
  const event = createArtilleryStartedEvent({
    agents,
    matchId: "durable-match",
    maxTurns: 8,
    timeoutMs: 15_000,
  });
  const content = appendArtilleryDurableEvent("Watch this match", event);

  assert.deepEqual(parseArtilleryDurableEvent(content), event);
  assert.equal(stripArtilleryDurableEvent(content), "Watch this match");
  assert.equal(parseArtilleryDurableEvent("ordinary message"), null);
});

test("recovers canonical turns and ignores duplicate or inconsistent events", () => {
  const started = createArtilleryStartedEvent({
    agents,
    matchId: "durable-match",
    maxTurns: 8,
    timeoutMs: 15_000,
  });
  const firstState = state(1, { red: 100, blue: 100 });
  const firstTurn = resolveArtilleryTurn(firstState, agents.red.name, {
    angle: 45,
    power: 72,
    weapon: "pulse-shell",
  });
  const secondState = state(2, {
    red: 100,
    blue: firstTurn.manifest.damage.after,
  });
  const secondTurn = resolveArtilleryTurn(secondState, agents.blue.name, {
    angle: 45,
    power: 72,
    weapon: "pulse-shell",
  });
  const firstResolved = createArtilleryTurnResolvedEvent(firstState, firstTurn);
  const secondResolved = createArtilleryTurnResolvedEvent(
    secondState,
    secondTurn,
  );
  const waiting = createArtilleryTurnRequestedEvent({
    agent: agents.red,
    deadlineAt: 20_000,
    requestId: "request-3",
    state: state(3, {
      red: secondTurn.manifest.damage.after,
      blue: firstTurn.manifest.damage.after,
    }),
  });

  const recovered = recoverArtilleryMatch(
    [secondResolved, started, firstResolved, firstResolved, waiting],
    "durable-match",
  );
  assert.equal(recovered?.match.turns.length, 2);
  assert.equal(recovered?.complete, false);
  assert.equal(recovered?.lastRequest?.requestId, "request-3");

  const finished = createArtilleryFinishedEvent(recovered.match);
  const complete = recoverArtilleryMatch(
    [finished, secondResolved, started, firstResolved],
    "durable-match",
  );
  assert.equal(complete?.complete, true);
  assert.equal(complete?.match.winner, recovered.match.winner);
});
