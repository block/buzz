import assert from "node:assert/strict";
import test from "node:test";

import {
  pickDirectProfileAgent,
  pickProfileAgent,
} from "./pickProfileAgent.ts";

test("the shared profile target prefers the active persona instance", () => {
  const stopped = {
    name: "Earlier instance",
    pubkey: "a".repeat(64),
    status: "stopped",
  };
  const running = {
    name: "Current instance",
    pubkey: "b".repeat(64),
    status: "running",
  };

  assert.equal(pickProfileAgent([stopped, running]), running);
  assert.equal(pickProfileAgent([running, stopped]), running);
});

test("a direct-opened active instance is never redirected to a sibling", () => {
  // "Alpha Sibling" sorts before "Tyler Agent"; without the direct guard an
  // access edit on Tyler would target the sibling.
  const sibling = {
    name: "Alpha Sibling",
    pubkey: "a".repeat(64),
    status: "running",
  };
  const clicked = {
    name: "Tyler Agent",
    pubkey: "b".repeat(64),
    status: "running",
  };

  assert.equal(pickDirectProfileAgent(clicked, [sibling, clicked]), clicked);
});

test("a direct-opened inactive instance redirects to the active sibling", () => {
  const historical = {
    name: "Earlier Parity Agent",
    pubkey: "a".repeat(64),
    status: "stopped",
  };
  const current = {
    name: "Current Parity Agent",
    pubkey: "b".repeat(64),
    status: "running",
  };

  assert.equal(
    pickDirectProfileAgent(historical, [historical, current]),
    current,
  );
});

test("a direct-opened inactive instance with no active sibling stays put", () => {
  const clicked = {
    name: "Only Instance",
    pubkey: "a".repeat(64),
    status: "stopped",
  };
  const otherStopped = {
    name: "Another Stopped",
    pubkey: "b".repeat(64),
    status: "stopped",
  };

  assert.equal(
    pickDirectProfileAgent(clicked, [clicked, otherStopped]),
    clicked,
  );
  assert.equal(pickDirectProfileAgent(clicked, []), clicked);
});
