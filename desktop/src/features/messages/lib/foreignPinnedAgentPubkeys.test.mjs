import assert from "node:assert/strict";
import test from "node:test";

import { foreignPinnedAgentPubkeys } from "./foreignPinnedAgentPubkeys.ts";

const A = "a".repeat(64);
const B = "b".repeat(64);
const C = "c".repeat(64);

test("instances pinned to another relay are foreign", () => {
  const foreign = foreignPinnedAgentPubkeys(
    [
      { pubkey: A, relayUrl: "wss://one.example" },
      { pubkey: B, relayUrl: "wss://two.example" },
    ],
    "wss://one.example",
  );
  assert.deepEqual([...foreign], [B]);
});

test("unpinned instances are never foreign", () => {
  const foreign = foreignPinnedAgentPubkeys(
    [
      { pubkey: A, relayUrl: "" },
      { pubkey: B, relayUrl: "   " },
      { pubkey: C, relayUrl: null },
    ],
    "wss://one.example",
  );
  assert.equal(foreign.size, 0);
});

test("pin comparison tolerates scheme, trailing slash, and case", () => {
  const foreign = foreignPinnedAgentPubkeys(
    [
      { pubkey: A, relayUrl: "one.example" },
      { pubkey: B, relayUrl: "WSS://One.Example/" },
      { pubkey: C, relayUrl: "wss://two.example/" },
    ],
    "wss://one.example",
  );
  assert.deepEqual([...foreign], [C]);
});

test("no active relay yields no foreign pins", () => {
  const agents = [{ pubkey: A, relayUrl: "wss://one.example" }];
  assert.equal(foreignPinnedAgentPubkeys(agents, null).size, 0);
  assert.equal(foreignPinnedAgentPubkeys(agents, "").size, 0);
  assert.equal(foreignPinnedAgentPubkeys(agents, undefined).size, 0);
});
