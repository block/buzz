import assert from "node:assert/strict";
import test from "node:test";

import { resolveThreadHarnessAgentPubkey } from "./threadHarnessTarget.ts";

const FIZZ = "aa".repeat(32);
const BUZZ = "bb".repeat(32);
const HUMAN = "cc".repeat(32);

test("returns null when the channel has no agents", () => {
  assert.equal(
    resolveThreadHarnessAgentPubkey({
      messages: [{ pubkey: HUMAN, tags: [["p", FIZZ]] }],
      agentPubkeys: [],
    }),
    null,
  );
});

test("returns null when no known agent appears in the thread", () => {
  assert.equal(
    resolveThreadHarnessAgentPubkey({
      messages: [{ pubkey: HUMAN, tags: [["p", HUMAN]] }],
      agentPubkeys: [FIZZ],
    }),
    null,
  );
});

test("resolves an agent mentioned by p tag", () => {
  assert.equal(
    resolveThreadHarnessAgentPubkey({
      messages: [{ pubkey: HUMAN, tags: [["p", FIZZ]] }],
      agentPubkeys: [FIZZ],
    }),
    FIZZ,
  );
});

test("resolves an agent that authored a message", () => {
  assert.equal(
    resolveThreadHarnessAgentPubkey({
      messages: [{ pubkey: FIZZ, tags: [] }],
      agentPubkeys: [FIZZ],
    }),
    FIZZ,
  );
});

test("ignores p tags naming pubkeys that are not known agents", () => {
  assert.equal(
    resolveThreadHarnessAgentPubkey({
      messages: [{ pubkey: HUMAN, tags: [["p", BUZZ]] }],
      agentPubkeys: [FIZZ],
    }),
    null,
  );
});

test("matches case-insensitively but returns the canonical pubkey", () => {
  assert.equal(
    resolveThreadHarnessAgentPubkey({
      messages: [{ pubkey: HUMAN, tags: [["p", FIZZ.toUpperCase()]] }],
      agentPubkeys: [FIZZ],
    }),
    FIZZ,
  );
});

test("prefers the earliest-appearing agent so the target stays stable", () => {
  assert.equal(
    resolveThreadHarnessAgentPubkey({
      messages: [
        { pubkey: HUMAN, tags: [["p", BUZZ]] },
        { pubkey: HUMAN, tags: [["p", FIZZ]] },
      ],
      agentPubkeys: [FIZZ, BUZZ],
    }),
    BUZZ,
  );
});

test("tolerates null messages, missing tags, and malformed tags", () => {
  assert.equal(
    resolveThreadHarnessAgentPubkey({
      messages: [null, undefined, {}, { tags: null }, { tags: [["p"]] }],
      agentPubkeys: [FIZZ],
    }),
    null,
  );
});
