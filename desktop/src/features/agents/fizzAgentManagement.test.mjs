import assert from "node:assert/strict";
import test from "node:test";

import {
  FIZZ_AGENT_MANAGEMENT_REQUEST,
  fizzRequestTargetsEditablePersona,
  parseFizzAgentManagementRequest,
} from "./fizzAgentManagement.ts";

const CHANNEL_ID = "7c07e659-3610-42f4-9a5e-1e9973c09da9";

function createPayload(overrides = {}) {
  return {
    type: FIZZ_AGENT_MANAGEMENT_REQUEST,
    action: "create",
    requestId: "request-1",
    request: {
      channelId: CHANNEL_ID,
      displayName: "Research helper",
      systemPrompt: "Find reliable sources and summarize them.",
      rationale: "The team needs faster research.",
    },
    ...overrides,
  };
}

test("parses the narrow no-secret create request", () => {
  assert.deepEqual(
    parseFizzAgentManagementRequest(createPayload()),
    createPayload(),
  );
});

test("rejects an agent-management request with extra secret-shaped fields", () => {
  const payload = createPayload();
  payload.request.apiKey = "should-not-be-accepted";

  assert.equal(parseFizzAgentManagementRequest(payload), null);
});

test("rejects allowlist access because selecting people stays in Desktop", () => {
  const payload = createPayload();
  payload.request.respondTo = "allowlist";

  assert.equal(parseFizzAgentManagementRequest(payload), null);
});

test("requires the originating channel for profile updates", () => {
  const payload = {
    type: FIZZ_AGENT_MANAGEMENT_REQUEST,
    action: "update",
    requestId: "request-2",
    request: {
      agentName: "Review helper",
      rationale: "Make its reviews more concise.",
      systemPrompt: "Review changes concisely.",
    },
  };

  assert.equal(parseFizzAgentManagementRequest(payload), null);
});

test("uses an agent's current name, never an internal profile ID", () => {
  const payload = {
    type: FIZZ_AGENT_MANAGEMENT_REQUEST,
    action: "update",
    requestId: "request-3",
    request: {
      channelId: CHANNEL_ID,
      agentName: "Review helper",
      rationale: "Make its reviews more concise.",
      systemPrompt: "Review changes concisely.",
    },
  };

  assert.deepEqual(parseFizzAgentManagementRequest(payload), payload);
});

test("allows Fizz to update only personal, editable profiles", () => {
  assert.equal(
    fizzRequestTargetsEditablePersona({ isBuiltIn: false, sourceTeam: null }),
    true,
  );
  assert.equal(
    fizzRequestTargetsEditablePersona({ isBuiltIn: true, sourceTeam: null }),
    false,
  );
  assert.equal(
    fizzRequestTargetsEditablePersona({ isBuiltIn: false, sourceTeam: "team" }),
    false,
  );
});
