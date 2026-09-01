/**
 * What the channel indicator lists, and what it claims.
 *
 * Provenance: the indicator showed a count and then opened whichever session
 * was newest, so a second live agent was named by the badge and reachable
 * nowhere in that control. The count itself was a session count wearing an
 * agent count's wording.
 */

import assert from "node:assert/strict";
import test from "node:test";

import {
  agentMediaEntries,
  describeAgentMediaEntries,
} from "./agentMediaEntries.ts";

const CARA = "a".repeat(64);
const RAY = "b".repeat(64);

const NAMES = { [CARA]: "Cara", [RAY]: "Ray" };
const labelFor = (pubkey) => NAMES[pubkey] ?? "unknown";

test("agentMediaEntries lists one entry per session, newest first", () => {
  const entries = agentMediaEntries(
    [{ agentPubkey: RAY }, { agentPubkey: CARA }],
    labelFor,
  );
  assert.deepEqual(entries, [
    { agentPubkey: RAY, label: "Ray" },
    { agentPubkey: CARA, label: "Cara" },
  ]);
});

test("agentMediaEntries collapses two sessions from one agent", () => {
  // The relay enforces no one-session-at-a-time rule, so an agent that
  // announces again before its first expires has two live starts. Both rows
  // would open the same panel.
  const entries = agentMediaEntries(
    [{ agentPubkey: CARA }, { agentPubkey: CARA }],
    labelFor,
  );
  assert.deepEqual(entries, [{ agentPubkey: CARA, label: "Cara" }]);
});

test("agentMediaEntries resolves a label once per agent", () => {
  const asked = [];
  agentMediaEntries(
    [{ agentPubkey: CARA }, { agentPubkey: CARA }, { agentPubkey: RAY }],
    (pubkey) => {
      asked.push(pubkey);
      return labelFor(pubkey);
    },
  );
  assert.deepEqual(asked, [CARA, RAY]);
});

test("describeAgentMediaEntries names a single agent and counts the rest", () => {
  assert.equal(describeAgentMediaEntries([]), "");
  assert.equal(
    describeAgentMediaEntries([{ agentPubkey: CARA, label: "Cara" }]),
    "Cara is live",
  );
  assert.equal(
    describeAgentMediaEntries([
      { agentPubkey: CARA, label: "Cara" },
      { agentPubkey: RAY, label: "Ray" },
    ]),
    "2 agents are live",
  );
});

test("describeAgentMediaEntries counts agents rather than sessions", () => {
  // Two sessions, one agent: "2 agents are live" would have been false.
  const entries = agentMediaEntries(
    [{ agentPubkey: CARA }, { agentPubkey: CARA }],
    labelFor,
  );
  assert.equal(describeAgentMediaEntries(entries), "Cara is live");
});
