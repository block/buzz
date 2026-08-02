import assert from "node:assert/strict";
import test from "node:test";

import { profileEditAgentTarget } from "./profileEditAgentTarget.ts";

function agent(overrides = {}) {
  return {
    backend: { type: "local" },
    agentCommand: "buzz-agent",
    agentArgs: [],
    personaId: "persona-1",
    ...overrides,
  };
}

const persona = { id: "persona-1", displayName: "Helper" };

test("a local persona-backed agent still opens the definition editor", () => {
  assert.equal(
    profileEditAgentTarget({ managedAgent: agent(), resolvedPersona: persona }),
    "definition",
  );
});

test("an agent with no definition opens the instance editor", () => {
  assert.equal(
    profileEditAgentTarget({
      managedAgent: agent({ personaId: null }),
      resolvedPersona: undefined,
    }),
    "instance",
  );
});

// The defect: every provider-created agent carries a personaId, so the persona
// branch swallowed the whole remote family — and the definition projection has
// no slot for backend/agent_command, so that dialog could only show a blank
// runtime and re-seed the local default.
test("a provider-backed record opens the instance editor despite its persona", () => {
  assert.equal(
    profileEditAgentTarget({
      managedAgent: agent({
        backend: { type: "provider", id: "ssh", config: {} },
        agentCommand: "/opt/homebrew/bin/goose",
        agentArgs: ["acp"],
      }),
      resolvedPersona: persona,
    }),
    "instance",
  );
});

test("only the backend decides — a local record with a host-shaped command stays local", () => {
  assert.equal(
    profileEditAgentTarget({
      managedAgent: agent({ agentCommand: "/opt/homebrew/bin/goose" }),
      resolvedPersona: persona,
    }),
    "definition",
    "providerRecordHarness stays the single owner of the remote question",
  );
});

test("a persona with no managed agent still opens the definition editor", () => {
  assert.equal(
    profileEditAgentTarget({
      managedAgent: undefined,
      resolvedPersona: persona,
    }),
    "definition",
  );
});
