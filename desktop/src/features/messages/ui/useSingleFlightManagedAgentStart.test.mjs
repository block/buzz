import assert from "node:assert/strict";
import test from "node:test";

import { managedAgentStartFlightKey } from "./useSingleFlightManagedAgentStart.ts";

const AGENT = "a".repeat(64);
const SIGNER = "b".repeat(64);

test("start flight keys coalesce only within the same tenant scope", () => {
  const scoped = managedAgentStartFlightKey({
    pubkey: AGENT,
    expectedRelayUrl: "wss://relay.example",
    expectedSignerPubkey: SIGNER,
  });

  assert.equal(
    scoped,
    managedAgentStartFlightKey({
      pubkey: AGENT.toUpperCase(),
      expectedRelayUrl: "wss://relay.example",
      expectedSignerPubkey: SIGNER.toUpperCase(),
    }),
  );
  assert.notEqual(
    scoped,
    managedAgentStartFlightKey({
      pubkey: AGENT,
      expectedRelayUrl: "wss://other.example",
      expectedSignerPubkey: SIGNER,
    }),
  );
  assert.notEqual(scoped, managedAgentStartFlightKey(AGENT));
});

test("a send's durable start shares the prewake's flight", () => {
  // Same agent, same tenant: the send must reuse the in-flight speculative
  // start rather than spawning a second one. Its own message dispatch is what
  // clears the harness's never-used bound.
  assert.equal(
    managedAgentStartFlightKey({
      pubkey: AGENT,
      expectedRelayUrl: "wss://relay.example",
      expectedSignerPubkey: SIGNER,
      speculative: true,
    }),
    managedAgentStartFlightKey({
      pubkey: AGENT,
      expectedRelayUrl: "wss://relay.example",
      expectedSignerPubkey: SIGNER,
    }),
  );
});
