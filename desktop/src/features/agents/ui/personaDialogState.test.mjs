import assert from "node:assert/strict";
import test from "node:test";

import {
  createPersonaDialogState,
  duplicatePersonaDialogState,
  editPersonaDialogState,
  importPersonaDialogState,
} from "./personaDialogState.ts";

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
    sourceFile: "import.persona.json",
  });

  assert.equal(state.title, "Import Imported");
  assert.deepEqual(state.initialValues, {
    displayName: "Imported",
    avatarUrl: "",
    systemPrompt: "Imported prompt",
    runtime: undefined,
    model: "model-b",
  });
});
