import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  agentBelongsToRelay,
  hasRunningAgentInCommunity,
  normalizeRelayUrlForCompare,
  partitionAgentsByRelay,
} from "./agentRelayScope.ts";

const RELAY_A = "ws://relay-a.example.com:3000";
const RELAY_B = "wss://relay-b.example.com";

// ── normalizeRelayUrlForCompare ──────────────────────────────────────────────
// Must agree with the Rust `normalize_relay_url` (desktop/src-tauri/src/relay.rs)
// because record pins are stamped by the backend and compared here. The
// agreement contract is the shared fixture below, consumed by both this file
// and the Rust unit tests (`normalize_agrees_with_shared_frontend_fixture`),
// so an edit to either normalizer fails the other side's tests instead of
// shipping a scoping skew.

test("normalize_agreesWithSharedBackendFixture", () => {
  const vectors = JSON.parse(
    readFileSync(
      new URL(
        "../../../fixtures/relay-url-normalization.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.ok(vectors.length > 0, "fixture must not be empty");
  for (const { input, canonical } of vectors) {
    assert.equal(
      normalizeRelayUrlForCompare(input),
      canonical,
      `input: ${JSON.stringify(input)}`,
    );
  }
});

test("normalize_stripsWhitespaceAndTrailingSlashes", () => {
  assert.equal(
    normalizeRelayUrlForCompare("  wss://relay.example.com//  "),
    "wss://relay.example.com",
  );
});

test("normalize_lowercasesSchemeAndAuthority_preservesPathCase", () => {
  assert.equal(
    normalizeRelayUrlForCompare("WSS://Relay.Example.COM:3000/Path"),
    "wss://relay.example.com:3000/Path",
  );
});

test("normalize_schemelessInput_passesThroughTrimmed", () => {
  assert.equal(normalizeRelayUrlForCompare("not-a-url/"), "not-a-url");
});

// ── agentBelongsToRelay ──────────────────────────────────────────────────────

test("belongs_exactMatch", () => {
  assert.equal(agentBelongsToRelay(RELAY_A, RELAY_A), true);
});

test("belongs_cosmeticDifferences_stillMatch", () => {
  // Trailing slash and scheme/host case must not split an agent from its
  // community — this is exactly what the shared normalizer exists for.
  assert.equal(
    agentBelongsToRelay("WS://RELAY-A.example.com:3000/", RELAY_A),
    true,
  );
});

test("belongs_differentRelay_doesNotMatch", () => {
  assert.equal(agentBelongsToRelay(RELAY_A, RELAY_B), false);
});

test("belongs_blankPin_followsActiveCommunity", () => {
  // Defense-in-depth mirror of the backend's `effective_agent_relay_url`
  // blank fallback: a record that escaped stamping follows the visited
  // community instead of vanishing from every community.
  assert.equal(agentBelongsToRelay("", RELAY_A), true);
  assert.equal(agentBelongsToRelay("   ", RELAY_B), true);
  assert.equal(agentBelongsToRelay(undefined, RELAY_A), true);
});

test("belongs_noActiveCommunityRelay_degradesToUnscoped", () => {
  assert.equal(agentBelongsToRelay(RELAY_A, null), true);
  assert.equal(agentBelongsToRelay(RELAY_A, ""), true);
  assert.equal(agentBelongsToRelay(RELAY_A, undefined), true);
});

// ── partitionAgentsByRelay: the agent-list filter ────────────────────────────

test("partition_scopesListToActiveCommunity", () => {
  // Two communities' agents coexist after lazy activation; the list the
  // user sees in community A must contain only A's agents (plus blank-pin
  // strays), with B's surfaced only through the "other communities" count.
  const agents = [
    { pubkey: "a1", relayUrl: RELAY_A },
    { pubkey: "b1", relayUrl: RELAY_B },
    { pubkey: "a2", relayUrl: `${RELAY_A}/` },
    { pubkey: "stray", relayUrl: "" },
  ];

  const { inCommunity, other } = partitionAgentsByRelay(agents, RELAY_A);

  assert.deepEqual(
    inCommunity.map((agent) => agent.pubkey),
    ["a1", "a2", "stray"],
  );
  assert.deepEqual(
    other.map((agent) => agent.pubkey),
    ["b1"],
  );
});

test("partition_undefinedAgents_yieldsEmpty", () => {
  const { inCommunity, other } = partitionAgentsByRelay(undefined, RELAY_A);
  assert.deepEqual(inCommunity, []);
  assert.deepEqual(other, []);
});

// ── hasRunningAgentInCommunity: the polling gate ─────────────────────────────

test("pollingGate_runningAgentInOtherCommunity_doesNotPoll", () => {
  // A workspace switch leaves the previous community's agents running; they
  // must not keep this community's 5s liveness poll alive.
  const agents = [
    { relayUrl: RELAY_B, status: "running" },
    { relayUrl: RELAY_A, status: "stopped" },
  ];

  assert.equal(hasRunningAgentInCommunity(agents, RELAY_A), false);
});

test("pollingGate_runningAgentInThisCommunity_polls", () => {
  const agents = [
    { relayUrl: RELAY_B, status: "running" },
    { relayUrl: RELAY_A, status: "running" },
  ];

  assert.equal(hasRunningAgentInCommunity(agents, RELAY_A), true);
});

test("pollingGate_blankPinRunningAgent_polls", () => {
  assert.equal(
    hasRunningAgentInCommunity([{ relayUrl: "", status: "running" }], RELAY_A),
    true,
  );
});
