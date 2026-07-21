import assert from "node:assert/strict";
import test from "node:test";

import {
  deriveModelDiscoveryPending,
  getDiscoveredPersonaModelOptions,
  isCacheableDiscoveryResponse,
  synthesizeEmptyDiscoveryStatus,
} from "./usePersonaModelDiscovery.ts";

function response(overrides = {}) {
  return {
    agentName: "mock",
    agentVersion: "0.0.0",
    models: [],
    agentDefaultModel: null,
    selectedModel: null,
    supportsSwitching: true,
    ...overrides,
  };
}

test("merges the harness's own 'default' catalog entry into the canonical default row", () => {
  const options = getDiscoveredPersonaModelOptions(
    response({
      models: [
        { id: "default", name: null, description: null },
        { id: "claude-opus-4-8", name: null, description: null },
        { id: "claude-sonnet-5", name: null, description: null },
      ],
    }),
    "",
  );

  // Exactly one default row (id ""), and no raw "default" entry remains.
  assert.deepEqual(
    options.map((option) => option.id),
    ["", "claude-opus-4-8", "claude-sonnet-5"],
  );
  assert.equal(options[0].label, "Default model");
});

test("default row shows the harness-reported current model when available", () => {
  const options = getDiscoveredPersonaModelOptions(
    response({
      agentDefaultModel: "gpt-5.5[high]",
      models: [
        { id: "gpt-5.5", name: "GPT-5.5", description: null },
        { id: "gpt-5.4", name: "GPT-5.4", description: null },
      ],
    }),
    "",
  );

  assert.equal(options[0].id, "");
  assert.equal(options[0].label, "Default model (gpt-5.5[high])");
  assert.deepEqual(
    options.slice(1).map((option) => option.id),
    ["gpt-5.5", "gpt-5.4"],
  );
});

test("the 'default' id match is case-insensitive and trimmed", () => {
  const options = getDiscoveredPersonaModelOptions(
    response({
      models: [
        { id: " Default ", name: null, description: null },
        { id: "claude-sonnet-5", name: null, description: null },
      ],
    }),
    "",
  );

  assert.deepEqual(
    options.map((option) => option.id),
    ["", "claude-sonnet-5"],
  );
});

test("explicit-model providers get no default row (no harness default entry)", () => {
  const options = getDiscoveredPersonaModelOptions(
    response({
      models: [
        { id: "goose-claude-4-6-sonnet", name: null, description: null },
      ],
    }),
    "anthropic",
  );

  assert.deepEqual(
    options.map((option) => option.id),
    ["goose-claude-4-6-sonnet"],
  );
});

test("relay-mesh keeps its automatic routing default row", () => {
  const options = getDiscoveredPersonaModelOptions(
    response({
      models: [{ id: "llama-3", name: "Llama 3", description: null }],
    }),
    "relay-mesh",
  );

  assert.equal(options[0].id, "");
  assert.equal(options[0].label, "Default (auto)");
});

test("returns null when discovery is unsupported or empty", () => {
  assert.equal(
    getDiscoveredPersonaModelOptions(
      response({ supportsSwitching: false }),
      "",
    ),
    null,
  );
  assert.equal(getDiscoveredPersonaModelOptions(null, ""), null);
});

// ── synthesizeEmptyDiscoveryStatus ────────────────────────────────────────────

test("synthesizeEmptyDiscoveryStatus_emptyModels_producesWarningStatus", () => {
  const status = synthesizeEmptyDiscoveryStatus(
    response({ models: [], agentName: "Claude Code" }),
    "",
  );
  assert.equal(status?.tone, "warning");
  assert.match(status?.message ?? "", /Claude Code/);
  assert.match(status?.message ?? "", /reported no models/);
});

test("synthesizeEmptyDiscoveryStatus_supportsSwitchingFalse_producesWarningStatus", () => {
  const status = synthesizeEmptyDiscoveryStatus(
    response({
      supportsSwitching: false,
      models: [{ id: "gpt-4", name: "GPT-4", description: null }],
      agentName: "Codex",
    }),
    "",
  );
  assert.equal(status?.tone, "warning");
  assert.match(status?.message ?? "", /Codex/);
});

test("synthesizeEmptyDiscoveryStatus_withUsableModels_returnsNull", () => {
  assert.equal(
    synthesizeEmptyDiscoveryStatus(
      response({
        models: [
          { id: "claude-sonnet-5", name: "Claude Sonnet 5", description: null },
        ],
        agentName: "Claude Code",
      }),
      "",
    ),
    null,
  );
});

test("synthesizeEmptyDiscoveryStatus_emptyAgentName_usesGenericFallback", () => {
  const status = synthesizeEmptyDiscoveryStatus(
    response({ models: [], agentName: "" }),
    "",
  );
  assert.equal(status?.tone, "warning");
  assert.match(status?.message ?? "", /This agent/);
});

// ── isCacheableDiscoveryResponse ──────────────────────────────────────────────

test("isCacheableDiscoveryResponse_withUsableModels_returnsTrue", () => {
  assert.equal(
    isCacheableDiscoveryResponse(
      response({
        models: [
          { id: "claude-sonnet-5", name: "Claude Sonnet 5", description: null },
        ],
      }),
      "",
    ),
    true,
  );
});

test("isCacheableDiscoveryResponse_emptyModels_returnsFalse", () => {
  // An empty-result response must not be cached so close→reopen retries
  // discovery after the user installs or signs into the CLI.
  assert.equal(
    isCacheableDiscoveryResponse(response({ models: [] }), ""),
    false,
  );
});

test("isCacheableDiscoveryResponse_supportsSwitchingFalse_returnsFalse", () => {
  assert.equal(
    isCacheableDiscoveryResponse(
      response({
        supportsSwitching: false,
        models: [{ id: "gpt-4", name: "GPT-4", description: null }],
      }),
      "",
    ),
    false,
  );
});

// ── deriveModelDiscoveryPending ────────────────────────────────────────────────

test("deriveModelDiscoveryPending_stillLoading_isTrue", () => {
  assert.equal(
    deriveModelDiscoveryPending({
      modelDiscoveryLoading: true,
      modelDiscoveryKey: "key",
      activeModelDiscoveryData: null,
      activeModelDiscoveryStatus: null,
    }),
    true,
  );
});

test("deriveModelDiscoveryPending_keySetDataNullStatusNull_isTrue", () => {
  // A key is set but neither data nor status has arrived yet → still pending.
  assert.equal(
    deriveModelDiscoveryPending({
      modelDiscoveryLoading: false,
      modelDiscoveryKey: "key",
      activeModelDiscoveryData: null,
      activeModelDiscoveryStatus: null,
    }),
    true,
  );
});

test("deriveModelDiscoveryPending_resolvedEmptyResponse_isNotPending", () => {
  // A resolved-but-empty response sets data non-null and status to a warning.
  // Neither condition for pending is met — the hook must not spin forever.
  const emptyResponse = response({ models: [] });
  const warningStatus = { message: "no models", tone: "warning" };
  assert.equal(
    deriveModelDiscoveryPending({
      modelDiscoveryLoading: false,
      modelDiscoveryKey: "key",
      activeModelDiscoveryData: emptyResponse,
      activeModelDiscoveryStatus: warningStatus,
    }),
    false,
  );
});

test("deriveModelDiscoveryPending_noKey_isNotPending", () => {
  // key=null means discovery is not expected (e.g. dialog closed).
  assert.equal(
    deriveModelDiscoveryPending({
      modelDiscoveryLoading: false,
      modelDiscoveryKey: null,
      activeModelDiscoveryData: null,
      activeModelDiscoveryStatus: null,
    }),
    false,
  );
});
