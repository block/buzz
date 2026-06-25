import assert from "node:assert/strict";
import test from "node:test";

import {
  canSubmitPersonaDialog,
  createPersonaDialogState,
  duplicatePersonaDialogState,
  editPersonaDialogState,
  importPersonaDialogState,
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

test("createPersonaDialogState returns a fresh empty draft", () => {
  const first = createPersonaDialogState();
  const second = createPersonaDialogState();

  assert.equal(first.title, "Create persona");
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

  assert.equal(state.title, "Edit persona");
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

test("importPersonaDialogState maps parsed persona previews into create drafts", () => {
  const state = importPersonaDialogState({
    displayName: "Imported",
    avatarDataUrl: null,
    systemPrompt: "Imported prompt",
    runtime: null,
    model: "model-b",
    provider: null,
    namePool: [],
    sourceFile: "import.persona.json",
  });

  assert.equal(state.title, "Import Imported");
  assert.deepEqual(state.initialValues, {
    displayName: "Imported",
    avatarUrl: "",
    systemPrompt: "Imported prompt",
    runtime: undefined,
    model: "model-b",
    provider: undefined,
  });
});

// ── Provider round-trip tests ─────────────────────────────────────────────────

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

test("importPersonaDialogState preserves provider=anthropic", () => {
  const state = importPersonaDialogState({
    displayName: "Imported With Provider",
    avatarDataUrl: null,
    systemPrompt: "Anthropic agent.",
    runtime: "goose",
    model: "claude-sonnet",
    provider: "anthropic",
    namePool: [],
    sourceFile: "provider-test.persona.json",
  });

  assert.equal(state.initialValues.provider, "anthropic");
});
