import assert from "node:assert/strict";
import test from "node:test";

import { buildBatchImportPersonaInput } from "./batchImportPersonaInput.ts";

function persona(overrides = {}) {
  return {
    displayName: "Imported Agent",
    avatarDataUrl: null,
    avatarRef: null,
    systemPrompt: "Use the imported provider.",
    runtime: "goose",
    model: "claude-sonnet-4",
    provider: "anthropic",
    namePool: [],
    sourceFile: "agent.persona.md",
    ...overrides,
  };
}

test("buildBatchImportPersonaInput preserves provider from parsed personas", () => {
  assert.deepEqual(buildBatchImportPersonaInput(persona()), {
    displayName: "Imported Agent",
    avatarUrl: undefined,
    systemPrompt: "Use the imported provider.",
    runtime: "goose",
    model: "claude-sonnet-4",
    provider: "anthropic",
    namePool: undefined,
  });
});

test("buildBatchImportPersonaInput carries imported name pools", () => {
  assert.deepEqual(
    buildBatchImportPersonaInput(persona({ namePool: ["fizz", "buzz"] })),
    {
      displayName: "Imported Agent",
      avatarUrl: undefined,
      systemPrompt: "Use the imported provider.",
      runtime: "goose",
      model: "claude-sonnet-4",
      provider: "anthropic",
      namePool: ["fizz", "buzz"],
    },
  );
});
