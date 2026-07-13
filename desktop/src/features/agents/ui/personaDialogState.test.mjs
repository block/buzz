import assert from "node:assert/strict";
import test from "node:test";

import {
  canSubmitPersonaDialog,
  createPersonaDialogState,
  duplicatePersonaDialogState,
  editPersonaDialogState,
  formatPersonaNamePoolText,
  parsePersonaNamePoolText,
} from "./personaDialogState.ts";

test("canSubmitPersonaDialog requires a display name but not a system prompt", () => {
  // Empty system prompt is allowed: core memory is auto-injected, so the
  // persona prompt is optional. Only the display name gates submission.
  assert.equal(
    canSubmitPersonaDialog({ displayName: "Coder", isPending: false }),
    true,
  );
  assert.equal(
    canSubmitPersonaDialog({ displayName: "  Coder  ", isPending: false }),
    true,
  );
});

test("canSubmitPersonaDialog blocks an empty or whitespace display name", () => {
  assert.equal(
    canSubmitPersonaDialog({ displayName: "", isPending: false }),
    false,
  );
  assert.equal(
    canSubmitPersonaDialog({ displayName: "   ", isPending: false }),
    false,
  );
});

test("canSubmitPersonaDialog blocks while a save is pending", () => {
  assert.equal(
    canSubmitPersonaDialog({ displayName: "Coder", isPending: true }),
    false,
  );
});

test("persona name pool helpers parse, format, and clear values", () => {
  assert.deepEqual(parsePersonaNamePoolText("Birch, Compass, , Ridge "), [
    "Birch",
    "Compass",
    "Ridge",
  ]);
  assert.deepEqual(parsePersonaNamePoolText("   "), []);
  assert.equal(
    formatPersonaNamePoolText(["Birch", "Compass", "Ridge"]),
    "Birch, Compass, Ridge",
  );
  assert.equal(formatPersonaNamePoolText(undefined), "");
});

test("createPersonaDialogState returns a fresh empty draft", () => {
  const first = createPersonaDialogState();
  const second = createPersonaDialogState();

  assert.equal(first.title, "Create agent");
  assert.deepEqual(first.initialValues, {
    displayName: "",
    avatarUrl: "",
    systemPrompt: "",
    runtime: undefined,
    model: undefined,
  });
  assert.notStrictEqual(first.initialValues, second.initialValues);
});

test("duplicatePersonaDialogState copies persona fields into a new draft", () => {
  const state = duplicatePersonaDialogState({
    id: "persona-1",
    displayName: "Solo",
    avatarUrl: "avatar://solo",
    systemPrompt: "Be direct.",
    runtime: "provider-a",
    model: "model-a",
    provider: null,
    isBuiltIn: false,
    isActive: true,
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-02T00:00:00Z",
  });

  assert.deepEqual(state.initialValues, {
    displayName: "Solo copy",
    avatarUrl: "avatar://solo",
    systemPrompt: "Be direct.",
    runtime: "provider-a",
    model: "model-a",
    provider: undefined,
    namePool: [],
    envVars: {},
  });
});

test("duplicatePersonaDialogState carries envVars and namePool into the duplicate", () => {
  // Regression: codex R10 P2. Without this, a duplicated persona that
  // relies on an API key in env_vars would silently fail at spawn until
  // the user re-entered every credential.
  const state = duplicatePersonaDialogState({
    id: "persona-with-secrets",
    displayName: "Coder",
    avatarUrl: null,
    systemPrompt: "Write code.",
    runtime: null,
    model: null,
    isBuiltIn: false,
    isActive: true,
    namePool: ["alice", "bob"],
    envVars: { ANTHROPIC_API_KEY: "sk-test", GOOSE_PROVIDER: "anthropic" },
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-02T00:00:00Z",
  });

  assert.deepEqual(state.initialValues.envVars, {
    ANTHROPIC_API_KEY: "sk-test",
    GOOSE_PROVIDER: "anthropic",
  });
  assert.deepEqual(state.initialValues.namePool, ["alice", "bob"]);
});

test("editPersonaDialogState preserves the persona id for updates", () => {
  const state = editPersonaDialogState({
    id: "persona-2",
    displayName: "Kit",
    avatarUrl: null,
    systemPrompt: "Keep it weird.",
    runtime: null,
    model: null,
    provider: null,
    isBuiltIn: true,
    isActive: true,
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-02T00:00:00Z",
  });

  assert.equal(state.title, "Edit agent");
  assert.equal(state.description, "");
  assert.equal(state.submitLabel, "Save changes");
  assert.deepEqual(state.initialValues, {
    id: "persona-2",
    displayName: "Kit",
    avatarUrl: "",
    systemPrompt: "Keep it weird.",
    runtime: undefined,
    model: undefined,
    provider: undefined,
    namePool: [],
    envVars: {},
  });
});

test("editPersonaDialogState seeds envVars and namePool from the persona", () => {
  const state = editPersonaDialogState({
    id: "persona-3",
    displayName: "Coder",
    avatarUrl: null,
    systemPrompt: "Write code.",
    runtime: null,
    model: null,
    isBuiltIn: false,
    isActive: true,
    namePool: ["alice", "bob"],
    envVars: { ANTHROPIC_API_KEY: "sk-test" },
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-02T00:00:00Z",
  });

  assert.deepEqual(state.initialValues.envVars, {
    ANTHROPIC_API_KEY: "sk-test",
  });
  assert.deepEqual(state.initialValues.namePool, ["alice", "bob"]);
});

test("editPersonaDialogState preserves provider=databricks", () => {
  const state = editPersonaDialogState({
    id: "persona-provider",
    displayName: "DB Agent",
    avatarUrl: null,
    systemPrompt: "Use databricks.",
    runtime: "goose",
    model: "dbrx",
    provider: "databricks",
    isBuiltIn: false,
    isActive: true,
    namePool: [],
    envVars: {},
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-02T00:00:00Z",
  });

  assert.equal(state.initialValues.provider, "databricks");
});

test("editPersonaDialogState maps provider=null to undefined", () => {
  const state = editPersonaDialogState({
    id: "persona-no-provider",
    displayName: "Plain",
    avatarUrl: null,
    systemPrompt: "No provider.",
    runtime: null,
    model: null,
    provider: null,
    isBuiltIn: false,
    isActive: true,
    namePool: [],
    envVars: {},
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-02T00:00:00Z",
  });

  assert.equal(state.initialValues.provider, undefined);
});

test("duplicatePersonaDialogState preserves provider=databricks", () => {
  const state = duplicatePersonaDialogState({
    id: "persona-dup-provider",
    displayName: "DB Agent",
    avatarUrl: null,
    systemPrompt: "Use databricks.",
    runtime: "goose",
    model: "dbrx",
    provider: "databricks",
    isBuiltIn: false,
    isActive: true,
    namePool: [],
    envVars: {},
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-02T00:00:00Z",
  });

  assert.equal(state.initialValues.provider, "databricks");
});

test("edit and duplicate seed the behavior group from a quad-bearing persona", () => {
  const persona = {
    id: "persona-quad",
    displayName: "Gated",
    avatarUrl: null,
    systemPrompt: "Guarded.",
    runtime: null,
    model: null,
    provider: null,
    isBuiltIn: false,
    isActive: true,
    respondTo: "allowlist",
    respondToAllowlist: ["a".repeat(64)],
    parallelism: 4,
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-02T00:00:00Z",
  };

  const expected = {
    respondTo: "allowlist",
    respondToAllowlist: ["a".repeat(64)],
    parallelism: 4,
  };
  assert.deepEqual(
    editPersonaDialogState(persona).initialValues.behavior,
    expected,
  );
  assert.deepEqual(
    duplicatePersonaDialogState(persona).initialValues.behavior,
    expected,
  );
});

test("a non-allowlist mode does not seed a stale allowlist into the dialog", () => {
  const state = editPersonaDialogState({
    id: "persona-mode-flip",
    displayName: "Open",
    avatarUrl: null,
    systemPrompt: "Open.",
    runtime: null,
    model: null,
    provider: null,
    isBuiltIn: false,
    isActive: true,
    respondTo: "owner-only",
    respondToAllowlist: ["b".repeat(64)],
    parallelism: null,
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-02T00:00:00Z",
  });

  assert.equal(state.initialValues.behavior.respondTo, "owner-only");
  assert.equal(
    state.initialValues.behavior.respondToAllowlist,
    undefined,
    "stale pubkeys must not resurrect through the dialog seed",
  );
});
