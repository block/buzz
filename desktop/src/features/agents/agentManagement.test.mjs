import assert from "node:assert/strict";
import test from "node:test";

import {
  AGENT_MANAGEMENT_REQUEST,
  createInputFromRequest,
  requestTargetsEditablePersona,
  parseAgentManagementRequest,
  resolveDeleteTarget,
  updateInputFromRequest,
} from "./agentManagement.ts";
import {
  behaviorForSubmit,
  draftFromBehavior,
} from "./ui/personaBehaviorDraft.ts";

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

test("agent-requested access edits preserve thread-scoped conversation context", () => {
  const request = {
    type: AGENT_MANAGEMENT_REQUEST,
    action: "update",
    requestId: "request-4",
    request: {
      channelId: CHANNEL_ID,
      agentName: "Review helper",
      respondTo: "anyone",
    },
  };
  const updated = updateInputFromRequest(request, {
    id: "review-helper",
    displayName: "Review helper",
    systemPrompt: "Review changes concisely.",
    behavior: {
      respondTo: "owner-only",
      respondToAllowlist: [],
      parallelism: 2,
      sessionPolicy: "thread",
    },
  });

  assert.equal(updated.behavior?.sessionPolicy, "thread");

  const seed = draftFromBehavior(updated.behavior);
  const edited = { ...seed, parallelism: "3" };
  assert.deepEqual(behaviorForSubmit(edited, seed, true), {
    respondTo: "anyone",
    respondToAllowlist: undefined,
    parallelism: 3,
    sessionPolicy: "thread",
  });
});

function deletePayload(overrides = {}) {
  return {
    type: AGENT_MANAGEMENT_REQUEST,
    action: "delete",
    requestId: "request-5",
    request: { channelId: CHANNEL_ID, agentName: "Scout" },
    ...overrides,
  };
}

test("parses a delete request that names only a channel and an agent", () => {
  assert.deepEqual(
    parseAgentManagementRequest(deletePayload()),
    deletePayload(),
  );
});

test("rejects malformed delete requests and unknown actions", () => {
  const withRequest = (request) => deletePayload({ request });
  for (const payload of [
    withRequest({ agentName: "Scout" }),
    withRequest({ channelId: CHANNEL_ID }),
    withRequest({ channelId: CHANNEL_ID, agentName: "  " }),
    withRequest({ channelId: CHANNEL_ID, agentName: "Scout", force: true }),
    withRequest({ channelId: CHANNEL_ID, agentName: "Scout", personaId: "p1" }),
    deletePayload({ requestId: "" }),
    deletePayload({ action: "remove" }),
    deletePayload({ action: "Delete" }),
    deletePayload({ action: undefined }),
    deletePayload({
      action: "remove",
      request: { channelId: CHANNEL_ID, agentName: "Scout", systemPrompt: "x" },
    }),
  ]) {
    assert.equal(parseAgentManagementRequest(payload), null);
  }
});

const persona = (overrides) => ({
  id: "p1",
  displayName: "Scout",
  isBuiltIn: false,
  sourceTeam: null,
  ...overrides,
});

test("delete resolves a single personal agent by its name, ignoring case", () => {
  const target = persona({ id: "p1", displayName: "Scout" });
  assert.deepEqual(
    resolveDeleteTarget(
      [persona({ id: "p2", displayName: "Other" }), target],
      " scout ",
    ),
    { status: "ready", persona: target },
  );
});

test("delete of an unknown, ambiguous, built-in, or team-managed agent is an error, never a no-op", () => {
  for (const [personas, pattern] of [
    [[persona({ displayName: "Other" })], /No personal agent is named “Scout”/],
    [[], /No personal agent is named “Scout”/],
    [[persona({ sourceTeam: "team" })], /No personal agent is named “Scout”/],
    [
      [persona({ id: "a" }), persona({ id: "b" })],
      /More than one personal agent/,
    ],
    [[persona({ isBuiltIn: true })], /built-in/],
  ]) {
    const target = resolveDeleteTarget(personas, "Scout");
    assert.equal(target.status, "error");
    assert.match(target.message, pattern);
  }
});

test("delete waits for the agent list instead of reporting an unknown agent", () => {
  assert.deepEqual(resolveDeleteTarget(undefined, "Scout"), {
    status: "loading",
  });
});
