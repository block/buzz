import assert from "node:assert/strict";
import test from "node:test";

import { personaCardEditAction } from "./personaCardEditAction.ts";

function agent(overrides = {}) {
  return {
    pubkey: "aa",
    name: "Scout",
    personaId: "persona-1",
    backend: { type: "local" },
    agentCommand: "buzz-agent",
    agentArgs: [],
    ...overrides,
  };
}

function persona(overrides = {}) {
  return {
    id: "persona-1",
    displayName: "Helper",
    isBuiltIn: false,
    ...overrides,
  };
}

const remote = agent({
  backend: { type: "provider", id: "ssh", config: {} },
  agentCommand: "/opt/homebrew/bin/goose",
  agentArgs: ["acp"],
});

test("a local persona-backed agent still opens the definition editor", () => {
  assert.deepEqual(personaCardEditAction(persona(), agent()), {
    type: "definition",
  });
});

test("a card with no linked record opens the definition editor", () => {
  assert.deepEqual(personaCardEditAction(persona(), undefined), {
    type: "definition",
  });
});

// The defect: the card's Edit reached the definition dialog unconditionally,
// even though the menu already held the record. The definition projection has
// no slot for backend/agent_command, so that dialog could only open on a blank
// runtime and re-seed this computer's default onto a remote agent.
test("a provider-backed record opens the instance editor on that record", () => {
  assert.deepEqual(personaCardEditAction(persona(), remote), {
    type: "instance",
    agent: remote,
    persona: persona(),
  });
});

test("only the backend decides — a local record with a host-shaped command stays local", () => {
  assert.deepEqual(
    personaCardEditAction(
      persona(),
      agent({ agentCommand: "/opt/homebrew/bin/goose", agentArgs: ["acp"] }),
    ),
    { type: "definition" },
    "providerRecordHarness stays the single owner of the remote question",
  );
});

test("the routed record carries its definition for the avatar hand-off", () => {
  const action = personaCardEditAction(persona(), remote);
  assert.equal(action.type, "instance");
  assert.equal(
    action.persona?.id,
    "persona-1",
    "the instance dialog cannot edit the shared avatar itself, so routing here must not strand the user without the definition",
  );
});

test("a built-in definition is not offered for editing", () => {
  assert.deepEqual(
    personaCardEditAction(persona({ isBuiltIn: true }), remote),
    {
      type: "instance",
      agent: remote,
      persona: null,
    },
  );
});

test("the card's own record is edited, ambiguity and all", () => {
  // pickProfileAgent already chose which of a persona's instances the card
  // renders, so this door edits what the user is looking at rather than
  // refusing the way agentManagementUpdateTarget does for a name out of chat.
  const second = agent({
    pubkey: "bb",
    name: "Scout 2",
    backend: { type: "provider", id: "ssh", config: {} },
    agentCommand: "/usr/bin/hermes",
    agentArgs: ["acp"],
  });
  const action = personaCardEditAction(persona(), second);
  assert.equal(action.type, "instance");
  assert.equal(action.agent, second);
});
