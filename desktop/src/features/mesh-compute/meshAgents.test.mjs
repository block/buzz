import assert from "node:assert/strict";
import test from "node:test";

/**
 * Contract tests for mesh agent detection and the card's call to action.
 *
 * Two things matter here. First, detection must read the **resolved** provider
 * off `ManagedAgent`: the mesh provider is usually set at the global layer, so
 * a persona's own `provider` field is null for most mesh agents — asking the
 * persona was the original bug. Second, the CTA's value is that it stays
 * quiet, so most of these tests pin *silence*.
 */

import { deriveMeshCallToAction, usesMeshCompute } from "./meshAgents.ts";

function agent(overrides = {}) {
  return { pubkey: "a1", provider: "relay-mesh", ...overrides };
}

test("detection reads the resolved provider, whitespace tolerant", () => {
  assert.equal(usesMeshCompute(agent()), true);
  assert.equal(usesMeshCompute(agent({ provider: " relay-mesh " })), true);
  assert.equal(usesMeshCompute(agent({ provider: "anthropic" })), false);
  // Null is the common case for an agent inheriting a NON-mesh global default,
  // and for an orphaned instance. Neither is a mesh agent.
  assert.equal(usesMeshCompute(agent({ provider: null })), false);
  assert.equal(usesMeshCompute(null), false);
  assert.equal(usesMeshCompute(undefined), false);
});

test("the CTA appears when there is compute and nothing using it", () => {
  // Sharing from this machine.
  assert.deepEqual(
    deriveMeshCallToAction({
      agents: [agent({ provider: "anthropic" })],
      isSharing: true,
      meshHasCapacity: false,
    }),
    { kind: "createAgent", label: "Set up an agent to use it" },
  );
  // Not sharing, but the community has capacity this person could use — a
  // legitimate reason to set up an agent even without contributing.
  assert.equal(
    deriveMeshCallToAction({
      agents: [],
      isSharing: false,
      meshHasCapacity: true,
    }).kind,
    "createAgent",
  );
});

test("the CTA disappears the moment a mesh agent exists", () => {
  assert.deepEqual(
    deriveMeshCallToAction({
      agents: [agent({ provider: "anthropic" }), agent({ pubkey: "a2" })],
      isSharing: true,
      meshHasCapacity: true,
    }),
    { kind: "none" },
  );
});

test("no capacity anywhere means the CTA would just move the dead end", () => {
  assert.deepEqual(
    deriveMeshCallToAction({
      agents: [],
      isSharing: false,
      meshHasCapacity: false,
    }),
    { kind: "none" },
  );
});

test("an unloaded agent list never prompts a possible duplicate", () => {
  // `undefined` is "not fetched yet". Prompting someone to create an agent
  // they may already have is worse than staying quiet for a moment.
  assert.deepEqual(
    deriveMeshCallToAction({
      agents: undefined,
      isSharing: true,
      meshHasCapacity: true,
    }),
    { kind: "none" },
  );
});
