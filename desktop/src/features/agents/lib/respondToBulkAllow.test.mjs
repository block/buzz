import assert from "node:assert/strict";
import test from "node:test";

import {
  bulkAllowHelperText,
  bulkAllowPubkeys,
  communityBotAllowlistPubkeys,
  isRespondToBulkAllowOption,
  localAgentAllowlistPubkeys,
} from "./respondToBulkAllow.ts";

const LOCAL_A = "a".repeat(64);
const LOCAL_B = "b".repeat(64);
const REMOTE_C = "c".repeat(64);
const BOT_D = "d".repeat(64);
const BOT_E = "E".repeat(64); // upper — normalize

const agents = [
  { pubkey: LOCAL_A, backend: { type: "local" } },
  { pubkey: LOCAL_B, backend: { type: "local" } },
  {
    pubkey: REMOTE_C,
    backend: { type: "provider", id: "blox", config: {} },
  },
];

const bots = [{ pubkey: BOT_D }, { pubkey: BOT_E }, { pubkey: BOT_D }];

test("isRespondToBulkAllowOption accepts only the three shortcuts", () => {
  assert.equal(isRespondToBulkAllowOption("all-local-agents"), true);
  assert.equal(isRespondToBulkAllowOption("all-community-bots"), true);
  assert.equal(isRespondToBulkAllowOption("all-bots"), true);
  assert.equal(isRespondToBulkAllowOption("allowlist"), false);
  assert.equal(isRespondToBulkAllowOption("anyone"), false);
});

test("localAgentAllowlistPubkeys keeps only local backends", () => {
  assert.deepEqual(localAgentAllowlistPubkeys(agents), [LOCAL_A, LOCAL_B]);
});

test("localAgentAllowlistPubkeys can exclude a pubkey", () => {
  assert.deepEqual(
    localAgentAllowlistPubkeys(agents, { excludePubkey: LOCAL_A }),
    [LOCAL_B],
  );
});

test("communityBotAllowlistPubkeys normalizes and dedupes", () => {
  assert.deepEqual(communityBotAllowlistPubkeys(bots), [
    BOT_D,
    BOT_E.toLowerCase(),
  ]);
});

test("bulkAllowPubkeys covers local, community, and all", () => {
  const input = { localAgents: agents, communityBots: bots };
  assert.deepEqual(bulkAllowPubkeys("all-local-agents", input), [
    LOCAL_A,
    LOCAL_B,
  ]);
  assert.deepEqual(bulkAllowPubkeys("all-community-bots", input), [
    BOT_D,
    BOT_E.toLowerCase(),
  ]);
  assert.deepEqual(bulkAllowPubkeys("all-bots", input), [
    LOCAL_A,
    LOCAL_B,
    BOT_D,
    BOT_E.toLowerCase(),
  ]);
});

test("bulkAllowHelperText names an empty set clearly", () => {
  assert.match(
    bulkAllowHelperText("all-local-agents", 0),
    /No local agents/,
  );
  assert.match(
    bulkAllowHelperText("all-community-bots", 0),
    /No community bots/,
  );
  assert.match(bulkAllowHelperText("all-bots", 0), /No local agents/);
});

test("bulkAllowHelperText reports the filled count", () => {
  assert.equal(
    bulkAllowHelperText("all-local-agents", 1),
    "Allows all 1 local agent on this computer.",
  );
  assert.equal(
    bulkAllowHelperText("all-community-bots", 2),
    "Allows all 2 installed community bots.",
  );
  assert.equal(
    bulkAllowHelperText("all-bots", 3),
    "Allows all 3 local agents and community bots.",
  );
});
