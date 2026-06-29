import assert from "node:assert/strict";
import test from "node:test";

import { buildPersonaImportUpdateInput } from "./personaImportUpdateInput.ts";

function createPersona(overrides = {}) {
  return {
    id: "persona-1",
    displayName: "Alice",
    avatarUrl: null,
    systemPrompt: "Be helpful.",
    runtime: "goose",
    model: "claude-sonnet-4",
    provider: "anthropic",
    namePool: [],
    isBuiltIn: false,
    isActive: true,
    envVars: {},
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function createPreview(overrides = {}) {
  return {
    displayName: "Alice",
    systemPrompt: "Be helpful.",
    avatarDataUrl: null,
    avatarRef: null,
    runtime: "goose",
    model: "gpt-5",
    provider: "databricks",
    namePool: [],
    sourceFile: "alice.md",
    ...overrides,
  };
}

test("buildPersonaImportUpdateInput applies selected model and provider updates", () => {
  const input = buildPersonaImportUpdateInput({
    existing: createPersona(),
    preview: createPreview(),
    selectedFields: ["model", "provider"],
  });

  assert.equal(input.model, "gpt-5");
  assert.equal(input.provider, "databricks");
});

test("buildPersonaImportUpdateInput preserves provider when provider is not selected", () => {
  const input = buildPersonaImportUpdateInput({
    existing: createPersona(),
    preview: createPreview(),
    selectedFields: ["model"],
  });

  assert.equal(input.model, "gpt-5");
  assert.equal(input.provider, "anthropic");
});
