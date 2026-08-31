import assert from "node:assert/strict";
import test from "node:test";

import { hermesTeamAgentForPersona } from "./hermesTeamBinding.ts";

const agent = (overrides = {}) => ({
  pubkey: "a".repeat(64),
  personaId: "persona-jake",
  runtime: "hermes",
  agentCommand: "hermes-acp",
  agentArgs: ["--profile", "jake"],
  teamId: null,
  ...overrides,
});

test("selects one exact profile-backed agent for the Team persona", () => {
  assert.equal(
    hermesTeamAgentForPersona({
      runtimeId: "hermes",
      personaId: "persona-jake",
      personaName: "Jake",
      teamId: "team-1",
      agents: [agent()],
    })?.pubkey,
    "a".repeat(64),
  );
});

test("rejects missing, duplicate, and cross-Team bindings", () => {
  assert.throws(
    () =>
      hermesTeamAgentForPersona({
        runtimeId: "hermes",
        personaId: "persona-jake",
        personaName: "Jake",
        teamId: "team-1",
        agents: [],
      }),
    /Connect its local profile/,
  );
  assert.throws(
    () =>
      hermesTeamAgentForPersona({
        runtimeId: "hermes",
        personaId: "persona-jake",
        personaName: "Jake",
        teamId: "team-1",
        agents: [agent(), agent({ pubkey: "b".repeat(64) })],
      }),
    /Multiple Buzz agents/,
  );
  assert.throws(
    () =>
      hermesTeamAgentForPersona({
        runtimeId: "hermes",
        personaId: "persona-jake",
        personaName: "Jake",
        teamId: "team-1",
        agents: [agent({ teamId: "team-2" })],
      }),
    /another Team/,
  );
});

test("non-Hermes runtimes need no singleton agent", () => {
  assert.equal(
    hermesTeamAgentForPersona({
      runtimeId: "buzz-agent",
      personaId: "reviewer",
      personaName: "Reviewer",
      teamId: "team-1",
      agents: [],
    }),
    null,
  );
});
