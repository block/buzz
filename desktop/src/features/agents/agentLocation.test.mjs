import assert from "node:assert/strict";
import test from "node:test";

import {
  LOCAL,
  resolveDefaultAgentLocation,
  sameLocation,
  setDefaultAgentLocation,
} from "./agentLocation.ts";

const SPAWNER_A = "aa".repeat(32);
const SPAWNER_B = "bb".repeat(32);

test("defaultsToLocal", () => {
  assert.deepEqual(resolveDefaultAgentLocation([]), LOCAL);
});

test("storesASpawnerAsTheDefault", () => {
  assert.equal(
    setDefaultAgentLocation({ kind: "spawner", spawnerPubkey: SPAWNER_A }),
    true,
  );
  assert.deepEqual(resolveDefaultAgentLocation([SPAWNER_A]), {
    kind: "spawner",
    spawnerPubkey: SPAWNER_A,
  });
});

test("fallsBackToLocalWhenTheDefaultSpawnerIsDisconnected", () => {
  // Otherwise every new agent would target a spawner this device no longer
  // manages and fail to deploy, with no obvious cause.
  setDefaultAgentLocation({ kind: "spawner", spawnerPubkey: SPAWNER_A });
  assert.deepEqual(resolveDefaultAgentLocation([SPAWNER_B]), LOCAL);
  // Reconnecting restores it: the preference is not destroyed by the fallback.
  assert.deepEqual(resolveDefaultAgentLocation([SPAWNER_A]), {
    kind: "spawner",
    spawnerPubkey: SPAWNER_A,
  });
});

test("rejectsAMalformedSpawnerPubkey", () => {
  assert.equal(
    setDefaultAgentLocation({ kind: "spawner", spawnerPubkey: "nope" }),
    false,
  );
});

test("normalisesCaseSoLookupsMatchConnectedSpawners", () => {
  setDefaultAgentLocation({
    kind: "spawner",
    spawnerPubkey: SPAWNER_A.toUpperCase(),
  });
  assert.deepEqual(resolveDefaultAgentLocation([SPAWNER_A]), {
    kind: "spawner",
    spawnerPubkey: SPAWNER_A,
  });
});

test("localCanBeSetBackExplicitly", () => {
  setDefaultAgentLocation({ kind: "spawner", spawnerPubkey: SPAWNER_A });
  setDefaultAgentLocation(LOCAL);
  assert.deepEqual(resolveDefaultAgentLocation([SPAWNER_A]), LOCAL);
});

test("sameLocationComparesKindAndSpawner", () => {
  assert.ok(sameLocation(LOCAL, LOCAL));
  assert.ok(
    sameLocation(
      { kind: "spawner", spawnerPubkey: SPAWNER_A },
      { kind: "spawner", spawnerPubkey: SPAWNER_A },
    ),
  );
  assert.ok(
    !sameLocation(
      { kind: "spawner", spawnerPubkey: SPAWNER_A },
      { kind: "spawner", spawnerPubkey: SPAWNER_B },
    ),
  );
  assert.ok(
    !sameLocation(LOCAL, { kind: "spawner", spawnerPubkey: SPAWNER_A }),
  );
});
