import assert from "node:assert/strict";
import test from "node:test";

import { collectPersonaRemoteCascadeInstances } from "./personaCascade.ts";

// Which cascade instances the persona-delete confirmation has to name. The
// answer drives both the warning copy and the backend opt-in, so an instance
// wrongly included prompts about nothing, and one wrongly excluded deletes a
// live deployment with no disclosure at all.

const PERSONA_ID = "custom:scout";

function instance(overrides = {}) {
  return {
    pubkey: "aa".repeat(32),
    name: "Instance",
    personaId: PERSONA_ID,
    backend: { type: "local" },
    backendAgentId: null,
    ...overrides,
  };
}

const deployed = instance({
  name: "Remote Scout",
  backend: { type: "provider", id: "blox", config: null },
  backendAgentId: "buzz-agent-scout.service",
});

test("collects provider instances that have a live deployment", () => {
  assert.deepEqual(
    collectPersonaRemoteCascadeInstances([deployed], PERSONA_ID),
    [{ name: "Remote Scout", unitId: "buzz-agent-scout.service" }],
  );
});

test("excludes local instances", () => {
  assert.deepEqual(
    collectPersonaRemoteCascadeInstances([instance()], PERSONA_ID),
    [],
  );
});

test("excludes provider instances that never deployed", () => {
  // No backend_agent_id means no deploy ever completed, so there is no remote
  // unit to leave running and nothing to warn about.
  const undeployed = instance({
    backend: { type: "provider", id: "blox", config: null },
  });
  assert.deepEqual(
    collectPersonaRemoteCascadeInstances([undeployed], PERSONA_ID),
    [],
  );
});

test("excludes deployed instances belonging to a different persona", () => {
  // These are not in the cascade, so naming them would warn about agents this
  // delete does not touch.
  const other = { ...deployed, personaId: "custom:other" };
  assert.deepEqual(
    collectPersonaRemoteCascadeInstances([other], PERSONA_ID),
    [],
  );
});

test("excludes instances with no persona", () => {
  const orphan = { ...deployed, personaId: null };
  assert.deepEqual(
    collectPersonaRemoteCascadeInstances([orphan], PERSONA_ID),
    [],
  );
});

test("collects every deployed instance in a mixed cascade", () => {
  const second = {
    ...deployed,
    name: "Relay Watcher",
    backendAgentId: "buzz-agent-relay.service",
  };
  assert.deepEqual(
    collectPersonaRemoteCascadeInstances(
      [
        instance(),
        deployed,
        { ...deployed, personaId: "custom:other" },
        second,
      ],
      PERSONA_ID,
    ),
    [
      { name: "Remote Scout", unitId: "buzz-agent-scout.service" },
      { name: "Relay Watcher", unitId: "buzz-agent-relay.service" },
    ],
  );
});
