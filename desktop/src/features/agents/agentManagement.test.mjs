import assert from "node:assert/strict";
import test from "node:test";

import {
  AGENT_MANAGEMENT_REQUEST,
  REQUIRED_NXTLINQ_SENSITIVE_EXCLUDES,
  createInputFromRequest,
  requestTargetsEditablePersona,
  parseAgentManagementRequest,
} from "./agentManagement.ts";

const CHANNEL_ID = "7c07e659-3610-42f4-9a5e-1e9973c09da9";

function nxtlinqPayload(capabilities, policyOverrides = {}) {
  const withRequiredConnection = capabilities.some(
    (capability) =>
      capability.type === "mcp:connect" &&
      Array.isArray(capability.servers) &&
      capability.servers.includes("buzz-dev-mcp"),
  )
    ? capabilities
    : [...capabilities, { type: "mcp:connect", servers: ["buzz-dev-mcp"] }];
  return {
    type: AGENT_MANAGEMENT_REQUEST,
    action: "nxtlinq_setup",
    requestId: "request-nxtlinq-strict",
    request: {
      channelId: CHANNEL_ID,
      projectRoot: "/workspace/project",
      explanation: "A narrowly scoped policy draft.",
      policy: {
        name: "review-agent",
        version: "1.0.0",
        scope: ["demo:structured-capabilities"],
        aud: ["nxtlinq-authorization-gateway"],
        capabilities: withRequiredConnection,
        ...policyOverrides,
      },
    },
  };
}

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

test("parses a policy-only Nxtlinq setup request", () => {
  const payload = {
    type: AGENT_MANAGEMENT_REQUEST,
    action: "nxtlinq_setup",
    requestId: "request-nxtlinq",
    request: {
      channelId: CHANNEL_ID,
      projectRoot: "/workspace/project",
      explanation: "Allow source reads while excluding secrets.",
      policy: {
        name: "review-agent",
        version: "1.0.0",
        scope: ["demo:structured-capabilities"],
        aud: ["nxtlinq-authorization-gateway"],
        capabilities: [
          {
            type: "filesystem:read",
            include: ["README.md", "src/**"],
            exclude: [...REQUIRED_NXTLINQ_SENSITIVE_EXCLUDES],
          },
          { type: "mcp:connect", servers: ["buzz-dev-mcp"] },
        ],
      },
    },
  };
  assert.deepEqual(parseAgentManagementRequest(payload), payload);
});

test("requires the exact inert Nxtlinq policy scope", () => {
  const capability = { type: "filesystem:read", include: ["src/**"] };

  for (const scope of [
    ["project:/workspace/project"],
    ["demo:structured-capabilities", "project:extra"],
    [],
  ]) {
    assert.equal(
      parseAgentManagementRequest(nxtlinqPayload([capability], { scope })),
      null,
    );
  }
});

test("requires the exact Nxtlinq gateway audience", () => {
  const capability = { type: "filesystem:read", include: ["src/**"] };

  for (const aud of [
    [],
    ["another-gateway"],
    ["nxtlinq-authorization-gateway", "another-audience"],
  ]) {
    assert.equal(
      parseAgentManagementRequest(nxtlinqPayload([capability], { aud })),
      null,
    );
  }
});

test("requires relative filesystem pattern arrays without parent traversal", () => {
  const emptyExclude = nxtlinqPayload([
    { type: "filesystem:read", include: ["src/**"], exclude: [] },
  ]);
  assert.equal(parseAgentManagementRequest(emptyExclude), null);

  const protectedRead = nxtlinqPayload([
    {
      type: "filesystem:read",
      include: ["src/**"],
      exclude: [...REQUIRED_NXTLINQ_SENSITIVE_EXCLUDES],
    },
  ]);
  assert.deepEqual(parseAgentManagementRequest(protectedRead), protectedRead);

  for (const include of [
    "/workspace/project/src/**",
    "C:\\workspace\\project\\src\\**",
    "\\\\server\\share\\src\\**",
    "../secrets/**",
    "src/../../secrets/**",
  ]) {
    assert.equal(
      parseAgentManagementRequest(
        nxtlinqPayload([{ type: "filesystem:read", include: [include] }]),
      ),
      null,
    );
  }

  for (const capability of [
    { type: "filesystem:read" },
    { type: "filesystem:read", include: "src/**" },
    { type: "filesystem:read", include: [] },
    {
      type: "filesystem:read",
      include: ["src/**"],
      exclude: ".env*",
    },
  ]) {
    assert.equal(
      parseAgentManagementRequest(nxtlinqPayload([capability])),
      null,
    );
  }
});

test("accepts only exact command arrays and environment variable names", () => {
  const valid = nxtlinqPayload([
    {
      type: "terminal:execute",
      commands: ["git status", "npm start"],
      environment: ["PATH", "_LOCAL_MODEL_2"],
      approvalRequired: false,
    },
  ]);
  assert.deepEqual(parseAgentManagementRequest(valid), valid);
  const emptyEnvironment = nxtlinqPayload([
    { type: "terminal:execute", commands: ["pwd"], environment: [] },
  ]);
  assert.equal(parseAgentManagementRequest(emptyEnvironment), null);

  for (const capability of [
    { type: "terminal:execute" },
    { type: "terminal:execute", commands: "git status" },
    { type: "terminal:execute", commands: [] },
    {
      type: "terminal:execute",
      commands: ["git status"],
      environment: "BUZZ_RELAY_URL",
    },
    {
      type: "terminal:execute",
      commands: ["git status"],
      environment: ["BUZZ_RELAY_URL=wss://relay.example"],
    },
    {
      type: "terminal:execute",
      commands: ["git status"],
      approvalRequired: 1,
    },
    {
      type: "terminal:execute",
      commands: ["git status"],
      approvalRequired: "true",
    },
    {
      type: "terminal:execute",
      commands: ["git status"],
      environment: ["PATH"],
      approvalRequired: true,
    },
    {
      type: "terminal:execute",
      commands: ["env"],
      environment: ["PATH", "BUZZ_PRIVATE_KEY"],
    },
  ]) {
    assert.equal(
      parseAgentManagementRequest(nxtlinqPayload([capability])),
      null,
    );
  }
});

test("requires canonical non-empty server and tool arrays for MCP", () => {
  const valid = nxtlinqPayload([
    { type: "mcp:connect", servers: ["external-tools"] },
    {
      type: "mcp:invoke",
      servers: ["external-tools"],
      tools: ["read_file"],
      approvalRequired: false,
    },
    {
      type: "mcp:invoke",
      servers: ["buzz-dev-mcp"],
      tools: ["view_image"],
    },
  ]);
  assert.deepEqual(parseAgentManagementRequest(valid), valid);

  for (const capability of [
    { type: "mcp:connect" },
    { type: "mcp:connect", servers: [] },
    { type: "mcp:connect", server: "external-tools" },
    {
      type: "mcp:connect",
      server: "external-tools",
      servers: ["external-tools"],
    },
    { type: "mcp:invoke", servers: ["external-tools"] },
    { type: "mcp:invoke", tools: ["search"] },
    {
      type: "mcp:invoke",
      servers: ["unconnected-tools"],
      tools: ["search"],
    },
    { type: "mcp:invoke", servers: [], tools: ["search"] },
    {
      type: "mcp:invoke",
      servers: ["external-tools", "buzz-dev-mcp"],
      tools: ["search"],
    },
    { type: "mcp:invoke", servers: ["external-tools"], tools: [] },
    {
      type: "mcp:invoke",
      server: "external-tools",
      tools: ["search"],
    },
    {
      type: "mcp:invoke",
      servers: ["external-tools"],
      tool: "search",
    },
    {
      type: "mcp:invoke",
      server: "external-tools",
      servers: ["external-tools"],
      tool: "search",
    },
    {
      type: "mcp:invoke",
      server: "external-tools",
      tool: "search",
      tools: ["search"],
    },
  ]) {
    assert.equal(
      parseAgentManagementRequest(nxtlinqPayload([capability])),
      null,
    );
  }
});

test("requires the Buzz MCP connection needed to create the Agent session", () => {
  const payload = nxtlinqPayload([
    { type: "filesystem:read", include: ["README.md"] },
  ]);
  payload.request.policy.capabilities =
    payload.request.policy.capabilities.filter(
      (capability) =>
        capability.type !== "mcp:connect" ||
        !capability.servers.includes("buzz-dev-mcp"),
    );
  assert.equal(parseAgentManagementRequest(payload), null);
});

test("rejects Buzz bundled semantic and control-plane tools as MCP grants", () => {
  for (const tool of [
    "read_file",
    "str_replace",
    "shell",
    "buzz_message_send",
    "nxtlinq_setup",
    "todo",
    "_Stop",
    "_PostCompact",
  ]) {
    assert.equal(
      parseAgentManagementRequest(
        nxtlinqPayload([
          {
            type: "mcp:invoke",
            servers: ["buzz-dev-mcp"],
            tools: [tool],
          },
        ]),
      ),
      null,
    );
  }
});

test("normalizes a legacy null Nxtlinq policy expiry", () => {
  const payload = {
    type: AGENT_MANAGEMENT_REQUEST,
    action: "nxtlinq_setup",
    requestId: "request-nxtlinq-null-exp",
    request: {
      channelId: CHANNEL_ID,
      projectRoot: "/workspace/project",
      explanation: "Allow source reads while excluding secrets.",
      policy: {
        name: "review-agent",
        version: "1.0.0",
        scope: ["demo:structured-capabilities"],
        aud: ["nxtlinq-authorization-gateway"],
        capabilities: [
          {
            type: "filesystem:read",
            include: ["README.md", "src/**"],
            exclude: [...REQUIRED_NXTLINQ_SENSITIVE_EXCLUDES],
          },
          { type: "mcp:connect", servers: ["buzz-dev-mcp"] },
        ],
        exp: null,
      },
    },
  };
  const parsed = parseAgentManagementRequest(payload);
  assert.ok(parsed);
  assert.equal(Object.hasOwn(parsed.request.policy, "exp"), false);
});

test("rejects private keys and unknown Nxtlinq capability constraints", () => {
  const base = {
    type: AGENT_MANAGEMENT_REQUEST,
    action: "nxtlinq_setup",
    requestId: "request-nxtlinq",
    request: {
      channelId: CHANNEL_ID,
      projectRoot: "/workspace/project",
      explanation: "A policy draft.",
      policy: {
        name: "review-agent",
        version: "1.0.0",
        scope: ["demo:structured-capabilities"],
        aud: ["nxtlinq-authorization-gateway"],
        capabilities: [
          { type: "filesystem:read", include: ["src/**"] },
          { type: "mcp:connect", servers: ["buzz-dev-mcp"] },
        ],
      },
    },
  };
  assert.equal(
    parseAgentManagementRequest({
      ...base,
      request: { ...base.request, privateKey: "secret" },
    }),
    null,
  );
  assert.equal(
    parseAgentManagementRequest({
      ...base,
      request: {
        ...base.request,
        policy: {
          ...base.request.policy,
          capabilities: [
            {
              type: "filesystem:read",
              include: ["src/**"],
              privateKey: "secret",
            },
            { type: "mcp:connect", servers: ["buzz-dev-mcp"] },
          ],
        },
      },
    }),
    null,
  );
});
