import assert from "node:assert/strict";
import test from "node:test";

import {
  agentLocationOptions,
  agentLocationValue,
  LOCAL_LOCATION_VALUE,
  parseAgentLocationValue,
} from "./agentLocationOptions.ts";

const SPAWNER_A = "aa".repeat(32);
const SPAWNER_B = "bb".repeat(32);

const describe = (pubkey) =>
  pubkey === SPAWNER_A
    ? { label: "GPU box", hint: "Claude Code" }
    : { label: "VPS" };

test("offersNoChoiceWhenNoSpawnerIsConnected", () => {
  // The caller hides the control entirely rather than rendering a dropdown
  // whose only entry is the thing that already happens.
  assert.deepEqual(agentLocationOptions([], describe), []);
});

test("listsThisMacFirstThenEachSpawner", () => {
  assert.deepEqual(agentLocationOptions([SPAWNER_A, SPAWNER_B], describe), [
    { value: LOCAL_LOCATION_VALUE, label: "This Mac" },
    { value: SPAWNER_A, label: "GPU box", hint: "Claude Code" },
    { value: SPAWNER_B, label: "VPS" },
  ]);
});

test("roundTripsALocationThroughTheSelectValue", () => {
  const location = { kind: "spawner", spawnerPubkey: SPAWNER_A };
  assert.equal(agentLocationValue(location), SPAWNER_A);
  assert.deepEqual(
    parseAgentLocationValue(agentLocationValue(location), [SPAWNER_A]),
    location,
  );
  assert.equal(agentLocationValue({ kind: "local" }), LOCAL_LOCATION_VALUE);
  assert.deepEqual(parseAgentLocationValue(LOCAL_LOCATION_VALUE, [SPAWNER_A]), {
    kind: "local",
  });
});

test("fallsBackToLocalForASpawnerThatIsNoLongerConnected", () => {
  // A spawner can disconnect while the dialog is open; deploying to it would
  // fail with no obvious cause.
  assert.deepEqual(parseAgentLocationValue(SPAWNER_A, [SPAWNER_B]), {
    kind: "local",
  });
  assert.deepEqual(parseAgentLocationValue("", [SPAWNER_A]), { kind: "local" });
});

test("acceptsAnUppercasePubkeyValue", () => {
  assert.deepEqual(
    parseAgentLocationValue(SPAWNER_A.toUpperCase(), [SPAWNER_A]),
    { kind: "spawner", spawnerPubkey: SPAWNER_A },
  );
});
