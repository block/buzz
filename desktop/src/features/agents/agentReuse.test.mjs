import assert from "node:assert/strict";
import test from "node:test";

import {
  commandsMatch,
  parseTimestamp,
  pickPreferredManagedAgent,
  findReusablePersonaAgent,
  findReusableGenericAgent,
  findReusableAgent,
} from "./agentReuse.ts";

const PUB_A = "a".repeat(64);
const PUB_B = "b".repeat(64);
const PUB_C = "c".repeat(64);

function makeAgent(overrides = {}) {
  return {
    id: "agent-1",
    pubkey: PUB_A,
    agentCommand: "goose",
    status: "running",
    personaId: null,
    systemPrompt: null,
    updatedAt: "2026-01-15T00:00:00Z",
    ...overrides,
  };
}

test("commandsMatch: bare names match", () => {
  assert.equal(commandsMatch("goose", "goose"), true);
});

test("commandsMatch: path variants match (unix path vs bare)", () => {
  assert.equal(commandsMatch("/usr/bin/goose", "goose"), true);
});

test("commandsMatch: backslash paths match", () => {
  assert.equal(commandsMatch("C:\\Users\\bin\\goose", "goose"), true);
});

test("commandsMatch: case insensitive", () => {
  assert.equal(commandsMatch("Goose", "GOOSE"), true);
});

test("commandsMatch: claude-code-acp normalizes to claude-acp", () => {
  assert.equal(commandsMatch("claude-code-acp", "claude-acp"), true);
});

test("commandsMatch: claude-agent-acp normalizes to claude-acp", () => {
  assert.equal(commandsMatch("claude-agent-acp", "claude-acp"), true);
});

test("commandsMatch: claude-code-acp matches claude-agent-acp", () => {
  assert.equal(commandsMatch("claude-code-acp", "claude-agent-acp"), true);
});

test("commandsMatch: path + claude normalization combined", () => {
  assert.equal(commandsMatch("/opt/bin/claude-code-acp", "claude-acp"), true);
});

test("commandsMatch: different commands do not match", () => {
  assert.equal(commandsMatch("goose", "claude-acp"), false);
});

test("parseTimestamp: valid ISO string", () => {
  const result = parseTimestamp("2026-01-15T00:00:00Z");
  assert.equal(result, Date.parse("2026-01-15T00:00:00Z"));
});

test("parseTimestamp: null returns 0", () => {
  assert.equal(parseTimestamp(null), 0);
});

test("parseTimestamp: undefined returns 0", () => {
  assert.equal(parseTimestamp(undefined), 0);
});

test("parseTimestamp: empty string returns 0", () => {
  assert.equal(parseTimestamp(""), 0);
});

test("parseTimestamp: invalid string returns 0", () => {
  assert.equal(parseTimestamp("not-a-date"), 0);
});

test("pickPreferredManagedAgent: empty array returns undefined", () => {
  assert.equal(pickPreferredManagedAgent([]), undefined);
});

test("pickPreferredManagedAgent: prefers running over stopped", () => {
  const running = makeAgent({
    id: "r",
    status: "running",
    updatedAt: "2025-01-01T00:00:00Z",
  });
  const stopped = makeAgent({
    id: "s",
    status: "stopped",
    updatedAt: "2026-06-01T00:00:00Z",
  });
  const result = pickPreferredManagedAgent([stopped, running]);
  assert.equal(result.id, "r");
});

test("pickPreferredManagedAgent: deployed treated same as running", () => {
  const deployed = makeAgent({
    id: "d",
    status: "deployed",
    updatedAt: "2025-01-01T00:00:00Z",
  });
  const stopped = makeAgent({
    id: "s",
    status: "stopped",
    updatedAt: "2026-06-01T00:00:00Z",
  });
  const result = pickPreferredManagedAgent([stopped, deployed]);
  assert.equal(result.id, "d");
});

test("pickPreferredManagedAgent: among same status, picks most recently updated", () => {
  const older = makeAgent({
    id: "old",
    status: "running",
    updatedAt: "2025-01-01T00:00:00Z",
  });
  const newer = makeAgent({
    id: "new",
    status: "running",
    updatedAt: "2026-06-01T00:00:00Z",
  });
  const result = pickPreferredManagedAgent([older, newer]);
  assert.equal(result.id, "new");
});

test("pickPreferredManagedAgent: null updatedAt treated as epoch 0", () => {
  const noTimestamp = makeAgent({
    id: "no-ts",
    status: "stopped",
    updatedAt: null,
  });
  const withTimestamp = makeAgent({
    id: "ts",
    status: "stopped",
    updatedAt: "2026-01-01T00:00:00Z",
  });
  const result = pickPreferredManagedAgent([noTimestamp, withTimestamp]);
  assert.equal(result.id, "ts");
});

test("pickPreferredManagedAgent: undefined updatedAt treated as epoch 0", () => {
  const noTimestamp = makeAgent({
    id: "no-ts",
    status: "stopped",
    updatedAt: undefined,
  });
  const withTimestamp = makeAgent({
    id: "ts",
    status: "stopped",
    updatedAt: "2025-06-01T00:00:00Z",
  });
  const result = pickPreferredManagedAgent([noTimestamp, withTimestamp]);
  assert.equal(result.id, "ts");
});

test("findReusablePersonaAgent: finds agent with matching personaId", () => {
  const agent = makeAgent({ personaId: "persona-1", pubkey: PUB_A });
  const channelMembers = new Set([PUB_B]);
  const result = findReusablePersonaAgent([agent], "persona-1", channelMembers);
  assert.equal(result, agent);
});

test("findReusablePersonaAgent: excludes agent already in channel", () => {
  const agent = makeAgent({ personaId: "persona-1", pubkey: PUB_A });
  const channelMembers = new Set([PUB_A]);
  const result = findReusablePersonaAgent([agent], "persona-1", channelMembers);
  assert.equal(result, undefined);
});

test("findReusablePersonaAgent: excludes agent with different personaId", () => {
  const agent = makeAgent({ personaId: "persona-2", pubkey: PUB_A });
  const channelMembers = new Set([PUB_B]);
  const result = findReusablePersonaAgent([agent], "persona-1", channelMembers);
  assert.equal(result, undefined);
});

test("findReusablePersonaAgent: prefers running agent", () => {
  const stopped = makeAgent({
    id: "s",
    personaId: "p1",
    pubkey: PUB_A,
    status: "stopped",
    updatedAt: "2026-06-01T00:00:00Z",
  });
  const running = makeAgent({
    id: "r",
    personaId: "p1",
    pubkey: PUB_B,
    status: "running",
    updatedAt: "2025-01-01T00:00:00Z",
  });
  const channelMembers = new Set([PUB_C]);
  const result = findReusablePersonaAgent(
    [stopped, running],
    "p1",
    channelMembers,
  );
  assert.equal(result.id, "r");
});

test("findReusablePersonaAgent: pubkey comparison is case-insensitive", () => {
  const agent = makeAgent({ personaId: "p1", pubkey: PUB_A.toUpperCase() });
  const channelMembers = new Set([PUB_A]);
  const result = findReusablePersonaAgent([agent], "p1", channelMembers);
  assert.equal(result, undefined);
});

test("findReusableGenericAgent: finds agent with matching command and no persona/prompt", () => {
  const agent = makeAgent({
    agentCommand: "goose",
    personaId: null,
    systemPrompt: null,
  });
  const channelMembers = new Set([PUB_B]);
  const result = findReusableGenericAgent([agent], "goose", channelMembers);
  assert.equal(result, agent);
});

test("findReusableGenericAgent: excludes agent with personaId", () => {
  const agent = makeAgent({
    agentCommand: "goose",
    personaId: "some-persona",
    systemPrompt: null,
  });
  const channelMembers = new Set([PUB_B]);
  const result = findReusableGenericAgent([agent], "goose", channelMembers);
  assert.equal(result, undefined);
});

test("findReusableGenericAgent: excludes agent with non-empty systemPrompt", () => {
  const agent = makeAgent({
    agentCommand: "goose",
    personaId: null,
    systemPrompt: "Do stuff",
  });
  const channelMembers = new Set([PUB_B]);
  const result = findReusableGenericAgent([agent], "goose", channelMembers);
  assert.equal(result, undefined);
});

test("findReusableGenericAgent: whitespace-only systemPrompt treated as empty (allowed)", () => {
  const agent = makeAgent({
    agentCommand: "goose",
    personaId: null,
    systemPrompt: "   \t\n  ",
  });
  const channelMembers = new Set([PUB_B]);
  const result = findReusableGenericAgent([agent], "goose", channelMembers);
  assert.equal(result, agent);
});

test("findReusableGenericAgent: undefined systemPrompt treated as empty (allowed)", () => {
  const agent = makeAgent({
    agentCommand: "goose",
    personaId: null,
    systemPrompt: undefined,
  });
  const channelMembers = new Set([PUB_B]);
  const result = findReusableGenericAgent([agent], "goose", channelMembers);
  assert.equal(result, agent);
});

test("findReusableGenericAgent: empty string systemPrompt treated as empty (allowed)", () => {
  const agent = makeAgent({
    agentCommand: "goose",
    personaId: null,
    systemPrompt: "",
  });
  const channelMembers = new Set([PUB_B]);
  const result = findReusableGenericAgent([agent], "goose", channelMembers);
  assert.equal(result, agent);
});

test("findReusableGenericAgent: excludes agent already in channel", () => {
  const agent = makeAgent({
    agentCommand: "goose",
    personaId: null,
    systemPrompt: null,
    pubkey: PUB_A,
  });
  const channelMembers = new Set([PUB_A]);
  const result = findReusableGenericAgent([agent], "goose", channelMembers);
  assert.equal(result, undefined);
});

test("findReusableGenericAgent: command matching uses normalization", () => {
  const agent = makeAgent({
    agentCommand: "/usr/local/bin/claude-code-acp",
    personaId: null,
    systemPrompt: null,
  });
  const channelMembers = new Set([PUB_B]);
  const result = findReusableGenericAgent(
    [agent],
    "claude-acp",
    channelMembers,
  );
  assert.equal(result, agent);
});

test("findReusableAgent: routes to persona search when personaId provided", () => {
  const agent = makeAgent({ personaId: "p1", pubkey: PUB_A });
  const channelMembers = new Set([PUB_B]);
  const result = findReusableAgent([agent], channelMembers, {
    personaId: "p1",
    command: "goose",
  });
  assert.equal(result, agent);
});

test("findReusableAgent: routes to generic search when no personaId and no prompt", () => {
  const agent = makeAgent({
    agentCommand: "goose",
    personaId: null,
    systemPrompt: null,
  });
  const channelMembers = new Set([PUB_B]);
  const result = findReusableAgent([agent], channelMembers, {
    command: "goose",
  });
  assert.equal(result, agent);
});

test("findReusableAgent: returns undefined when systemPrompt is non-empty (custom agent)", () => {
  const agent = makeAgent({
    agentCommand: "goose",
    personaId: null,
    systemPrompt: null,
  });
  const channelMembers = new Set([PUB_B]);
  const result = findReusableAgent([agent], channelMembers, {
    command: "goose",
    systemPrompt: "Custom instructions",
  });
  assert.equal(result, undefined);
});

test("findReusableAgent: whitespace-only systemPrompt in input still routes to generic", () => {
  const agent = makeAgent({
    agentCommand: "goose",
    personaId: null,
    systemPrompt: null,
  });
  const channelMembers = new Set([PUB_B]);
  const result = findReusableAgent([agent], channelMembers, {
    command: "goose",
    systemPrompt: "   ",
  });
  assert.equal(result, agent);
});

test("findReusableAgent: null personaId in input routes to generic", () => {
  const agent = makeAgent({
    agentCommand: "goose",
    personaId: null,
    systemPrompt: null,
  });
  const channelMembers = new Set([PUB_B]);
  const result = findReusableAgent([agent], channelMembers, {
    personaId: null,
    command: "goose",
  });
  assert.equal(result, agent);
});
