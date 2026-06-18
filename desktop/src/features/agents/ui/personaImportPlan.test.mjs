import assert from "node:assert/strict";
import test from "node:test";

import {
  buildPersonaImportPlan,
  hasAnyPersonaImportChanges,
} from "./personaImportPlan.ts";

function createPersona(overrides = {}) {
  return {
    id: "persona-1",
    displayName: "Alice",
    avatarUrl: null,
    systemPrompt: "Be helpful.",
    runtime: null,
    model: null,
    namePool: [],
    isBuiltIn: false,
    isActive: true,
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
    runtime: null,
    model: null,
    namePool: [],
    sourceFile: "alice.persona.json",
    ...overrides,
  };
}

test("buildPersonaImportPlan returns empty fields when nothing changed", () => {
  const plan = buildPersonaImportPlan({
    persona: createPersona(),
    preview: createPreview(),
  });

  assert.equal(plan.fields.length, 0);
});

test("buildPersonaImportPlan detects display name change", () => {
  const plan = buildPersonaImportPlan({
    persona: createPersona({ displayName: "Alice" }),
    preview: createPreview({ displayName: "Alicia" }),
  });

  assert.equal(plan.fields.length, 1);
  assert.equal(plan.fields[0]?.field, "displayName");
  assert.equal(plan.fields[0]?.label, "Display name");
  assert.equal(plan.fields[0]?.existingValue, "Alice");
  assert.equal(plan.fields[0]?.importedValue, "Alicia");
  assert.equal(plan.fields[0]?.addedLines, 1);
  assert.equal(plan.fields[0]?.removedLines, 1);
});

test("buildPersonaImportPlan detects system prompt change with line counts", () => {
  const plan = buildPersonaImportPlan({
    persona: createPersona({ systemPrompt: "Line one\nLine two" }),
    preview: createPreview({
      systemPrompt: "Line one\nLine two\nLine three",
    }),
  });

  assert.equal(plan.fields.length, 1);
  assert.equal(plan.fields[0]?.field, "systemPrompt");
  assert.equal(plan.fields[0]?.addedLines, 1);
  assert.equal(plan.fields[0]?.removedLines, 0);
});

test("buildPersonaImportPlan detects avatar change", () => {
  const plan = buildPersonaImportPlan({
    persona: createPersona({ avatarUrl: "https://old.png" }),
    preview: createPreview({ avatarDataUrl: "https://new.png" }),
  });

  assert.equal(plan.fields.length, 1);
  assert.equal(plan.fields[0]?.field, "avatarUrl");
});

test("buildPersonaImportPlan detects runtime change", () => {
  const plan = buildPersonaImportPlan({
    persona: createPersona({ runtime: "goose" }),
    preview: createPreview({ runtime: "claude" }),
  });

  assert.equal(plan.fields.length, 1);
  assert.equal(plan.fields[0]?.field, "runtime");
  assert.equal(plan.fields[0]?.label, "Preferred runtime");
});

test("buildPersonaImportPlan detects model change", () => {
  const plan = buildPersonaImportPlan({
    persona: createPersona({ model: "gpt-4o" }),
    preview: createPreview({ model: "claude-sonnet-4-20250514" }),
  });

  assert.equal(plan.fields.length, 1);
  assert.equal(plan.fields[0]?.field, "model");
  assert.equal(plan.fields[0]?.label, "Preferred model");
});

test("buildPersonaImportPlan detects name pool change", () => {
  const plan = buildPersonaImportPlan({
    persona: createPersona({ namePool: ["Birch", "Compass"] }),
    preview: createPreview({ namePool: ["Ridge", "Thistle"] }),
  });

  assert.equal(plan.fields.length, 1);
  assert.equal(plan.fields[0]?.field, "namePool");
  assert.equal(plan.fields[0]?.label, "Instance name pool");
});

test("buildPersonaImportPlan detects multiple field changes", () => {
  const plan = buildPersonaImportPlan({
    persona: createPersona({
      displayName: "Alice",
      systemPrompt: "Old prompt",
      model: "gpt-4o",
    }),
    preview: createPreview({
      displayName: "Alicia",
      systemPrompt: "New prompt",
      model: "claude-sonnet-4-20250514",
    }),
  });

  assert.equal(plan.fields.length, 3);
  const fieldNames = plan.fields.map((f) => f.field);
  assert.deepEqual(fieldNames, ["displayName", "systemPrompt", "model"]);
});

test("hasAnyPersonaImportChanges returns false for empty plan", () => {
  assert.equal(hasAnyPersonaImportChanges({ fields: [] }), false);
  assert.equal(hasAnyPersonaImportChanges(null), false);
});

test("hasAnyPersonaImportChanges returns true when fields have changes", () => {
  assert.equal(
    hasAnyPersonaImportChanges({
      fields: [
        {
          field: "displayName",
          label: "Display name",
          existingValue: "A",
          importedValue: "B",
          addedLines: 1,
          removedLines: 1,
        },
      ],
    }),
    true,
  );
});
