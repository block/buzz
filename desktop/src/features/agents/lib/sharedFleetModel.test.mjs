import assert from "node:assert/strict";
import test from "node:test";

import {
  activeSharedFleetRows,
  buildSharedFleetRows,
} from "./sharedFleetModel.ts";

const PUB_A = "a".repeat(64);
const PUB_B = "b".repeat(64);

function agent(overrides = {}) {
  return {
    pubkey: PUB_A,
    name: "Remote Scout",
    model: null,
    agentType: "agent",
    channels: ["Agent Testing", "Agent Testing", "x-articles-ozark"],
    channelIds: ["a", "b"],
    capabilities: [],
    status: "offline",
    respondTo: "anyone",
    respondToAllowlist: [],
    ...overrides,
  };
}

test("current presence overrides directory status and deduplicates channels", () => {
  const rows = buildSharedFleetRows([agent({ model: "Model Alpha" })], {
    [PUB_A]: "online",
  });
  assert.deepEqual(rows, [
    {
      pubkey: PUB_A,
      name: "Remote Scout",
      modelLabel: "Model Alpha",
      status: "online",
      channels: ["Agent Testing", "x-articles-ozark"],
      mentionHint: "Mention only in its 2 assigned channels",
    },
  ]);
});

test("offline and directory-only records are excluded from active fleet", () => {
  const rows = buildSharedFleetRows(
    [
      agent(),
      agent({ pubkey: PUB_B, name: "Offline Remote", status: "online" }),
    ],
    { [PUB_A]: "away", [PUB_B]: "offline" },
  );
  assert.deepEqual(
    activeSharedFleetRows(rows).map((row) => row.name),
    ["Remote Scout"],
  );

  const directoryOnly = buildSharedFleetRows(
    [agent({ status: "online" })],
    undefined,
  );
  assert.deepEqual(activeSharedFleetRows(directoryOnly), []);
});

test("unknown presence cannot make a worker live", () => {
  const rows = buildSharedFleetRows([agent()], { [PUB_A]: "transitioning" });
  assert.deepEqual(activeSharedFleetRows(rows), []);
});
