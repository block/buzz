import assert from "node:assert/strict";
import test from "node:test";

import {
  AGENT_MANAGEMENT_REQUEST,
  createInputFromRequest,
  requestTargetsEditablePersona,
  parseAgentManagementRequest,
} from "./agentManagement.ts";

const CHANNEL_ID = "7c07e659-3610-42f4-9a5e-1e9973c09da9";

function createPayload(overrides = {}) {
  return {
    type: AGENT_MANAGEMENT_REQUEST,
    action: "create",
    requestId: "request-1",
    request: {
      channelId: CHANNEL_ID,
      displayName: "Research helper",
      systemPrompt: "Find reliable sources and summarize them.",
    },
    ...overrides,
  };
}

test("parses the narrow no-secret create request", () => {
  assert.deepEqual(
    parseAgentManagementRequest(createPayload()),
    createPayload(),
  );
});

test("rejects an agent-management request with extra secret-shaped fields", () => {
  const payload = createPayload();
  payload.request.apiKey = "should-not-be-accepted";

  assert.equal(parseAgentManagementRequest(payload), null);
});

test("chat creation cannot choose runtime, provider, model, or access", () => {
  for (const [field, value] of [
    ["runtime", "claude"],
    ["provider", "anthropic"],
    ["model", "claude-opus"],
    ["respondTo", "anyone"],
  ]) {
    const payload = createPayload();
    payload.request[field] = value;
    assert.equal(parseAgentManagementRequest(payload), null);
  }
});

test("chat creation leaves advanced behavior unset so the form stays collapsed", () => {
  const parsed = parseAgentManagementRequest(createPayload());
  assert.ok(parsed && parsed.action === "create");

  assert.deepEqual(createInputFromRequest(parsed), {
    displayName: "Research helper",
    systemPrompt: "Find reliable sources and summarize them.",
  });
});

test("requires the originating channel for profile updates", () => {
  const payload = {
    type: AGENT_MANAGEMENT_REQUEST,
    action: "update",
    requestId: "request-2",
    request: {
      agentName: "Review helper",
      systemPrompt: "Review changes concisely.",
    },
  };

  assert.equal(parseAgentManagementRequest(payload), null);
});

test("uses an agent's current name, never an internal profile ID", () => {
  const payload = {
    type: AGENT_MANAGEMENT_REQUEST,
    action: "update",
    requestId: "request-3",
    request: {
      channelId: CHANNEL_ID,
      agentName: "Review helper",
      systemPrompt: "Review changes concisely.",
    },
  };

  assert.deepEqual(parseAgentManagementRequest(payload), payload);
});

test("uses instance pubkey when display name is ambiguous", () => {
  const pubkey = "a".repeat(64);
  const payload = {
    type: AGENT_MANAGEMENT_REQUEST,
    action: "update",
    requestId: "request-pk",
    request: {
      channelId: CHANNEL_ID,
      agentPubkey: pubkey,
      systemPrompt: "Updated.",
    },
  };

  assert.deepEqual(parseAgentManagementRequest(payload), {
    ...payload,
    request: { ...payload.request, agentName: "" },
  });
});

test("findPersonaAgentInChannel: returns instance already in channel", async () => {
  const { findPersonaAgentInChannel } = await import("./agentReuse.ts");
  const PUB_A = "a".repeat(64);
  const agent = {
    id: "agent-1",
    pubkey: PUB_A,
    agentCommand: "goose",
    status: "running",
    personaId: "persona-1",
    systemPrompt: null,
    updatedAt: "2026-01-15T00:00:00Z",
  };
  const channelMembers = new Set([PUB_A]);
  assert.equal(
    findPersonaAgentInChannel([agent], "persona-1", channelMembers)?.id,
    "agent-1",
  );
});

test("findPersonaAgentInChannel: excludes agent not in channel", async () => {
  const { findPersonaAgentInChannel } = await import("./agentReuse.ts");
  const PUB_A = "a".repeat(64);
  const PUB_B = "b".repeat(64);
  const agent = {
    id: "agent-1",
    pubkey: PUB_A,
    agentCommand: "goose",
    status: "running",
    personaId: "persona-1",
    systemPrompt: null,
    updatedAt: "2026-01-15T00:00:00Z",
  };
  const channelMembers = new Set([PUB_B]);
  assert.equal(
    findPersonaAgentInChannel([agent], "persona-1", channelMembers),
    undefined,
  );
});

test("allows agents to update only personal, editable profiles", () => {
  assert.equal(
    requestTargetsEditablePersona({ isBuiltIn: false, sourceTeam: null }),
    true,
  );
  assert.equal(
    requestTargetsEditablePersona({ isBuiltIn: true, sourceTeam: null }),
    true,
  );
  assert.equal(
    requestTargetsEditablePersona({ isBuiltIn: false, sourceTeam: "team" }),
    false,
  );
});
