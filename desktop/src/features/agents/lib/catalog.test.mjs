import assert from "node:assert/strict";
import test from "node:test";

import {
  getLibraryPersonas,
  getPersonaLabelsById,
  isCatalogPersonaSelected,
} from "./catalog.ts";

function createPersona(id, displayName, overrides = {}) {
  return {
    id,
    displayName,
    avatarUrl: overrides.avatarUrl ?? null,
    systemPrompt: overrides.systemPrompt ?? `${displayName} prompt`,
    runtime: overrides.runtime ?? null,
    model: overrides.model ?? null,
    isBuiltIn: overrides.isBuiltIn ?? false,
    isActive: overrides.isActive ?? true,
    createdAt: overrides.createdAt ?? "2026-01-01T00:00:00Z",
    updatedAt: overrides.updatedAt ?? "2026-01-01T00:00:00Z",
  };
}

test("isCatalogPersonaSelected treats active catalog personas as selected", () => {
  assert.equal(
    isCatalogPersonaSelected(
      createPersona("builtin:fizz", "Fizz", {
        isBuiltIn: true,
        isActive: true,
      }),
    ),
    true,
  );
  assert.equal(
    isCatalogPersonaSelected(
      createPersona("builtin:fizz", "Fizz", {
        isBuiltIn: true,
        isActive: false,
      }),
    ),
    false,
  );
  assert.equal(
    isCatalogPersonaSelected(createPersona("custom:builder", "Builder")),
    true,
  );
});

test("getPersonaLabelsById keeps every returned persona addressable", () => {
  const personas = [
    createPersona("builtin:fizz", "Fizz", { isBuiltIn: true, isActive: false }),
    createPersona("custom:builder", "Builder"),
  ];

  assert.deepEqual(getPersonaLabelsById(personas), {
    "builtin:fizz": "Fizz",
    "custom:builder": "Builder",
  });
});

test("getLibraryPersonas keeps active custom personas even when catalog entries are similar", () => {
  const avatarUrl = "https://example.test/coordinator.png";
  const personas = [
    createPersona("builtin:work-coordinator", "Work Coordinator", {
      avatarUrl,
      isBuiltIn: true,
      isActive: false,
    }),
    createPersona("custom:work-coordinator", "Work Coordinator", {
      avatarUrl,
      isActive: true,
    }),
    createPersona("custom:builder", "Builder", { isActive: true }),
  ];

  assert.deepEqual(
    getLibraryPersonas(personas).map((persona) => persona.id),
    ["custom:work-coordinator", "custom:builder"],
  );
});
