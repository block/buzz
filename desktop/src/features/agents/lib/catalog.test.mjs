import assert from "node:assert/strict";
import test from "node:test";

import {
  getCatalogPersonas,
  getCatalogSelectionState,
  getPersonaLabelsById,
  getPersonaLibraryState,
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

test("getCatalogPersonas keeps built-ins visible whether selected or not", () => {
  const personas = [
    createPersona("builtin:solo", "Solo", { isBuiltIn: true, isActive: false }),
    createPersona("builtin:kit", "Kit", {
      isBuiltIn: true,
      isActive: true,
    }),
    createPersona("custom:builder", "Builder"),
  ];

  assert.deepEqual(
    getCatalogPersonas(personas).map((persona) => persona.id),
    ["builtin:kit", "builtin:solo"],
  );
});

test("getCatalogSelectionState keeps built-in selection rules in one place", () => {
  const personas = [
    createPersona("builtin:solo", "Solo", { isBuiltIn: true, isActive: false }),
    createPersona("builtin:kit", "Kit", {
      isBuiltIn: true,
      isActive: true,
    }),
    createPersona("custom:builder", "Builder"),
  ];

  const state = getCatalogSelectionState(personas);

  assert.deepEqual(
    state.catalogPersonas.map((persona) => persona.id),
    ["builtin:kit", "builtin:solo"],
  );
  assert.deepEqual(
    state.selectedCatalogPersonas.map((persona) => persona.id),
    ["builtin:kit"],
  );
  assert.deepEqual(
    state.unselectedCatalogPersonas.map((persona) => persona.id),
    ["builtin:solo"],
  );
});

test("getCatalogPersonas keeps chooser order stable when selection changes", () => {
  const inactiveFirst = [
    createPersona("builtin:solo", "Solo", { isBuiltIn: true, isActive: false }),
    createPersona("builtin:kit", "Kit", {
      isBuiltIn: true,
      isActive: true,
    }),
    createPersona("builtin:reviewer", "Reviewer", {
      isBuiltIn: true,
      isActive: false,
    }),
  ];
  const activeFirst = [
    createPersona("builtin:solo", "Solo", { isBuiltIn: true, isActive: true }),
    createPersona("builtin:kit", "Kit", {
      isBuiltIn: true,
      isActive: false,
    }),
    createPersona("builtin:reviewer", "Reviewer", {
      isBuiltIn: true,
      isActive: true,
    }),
  ];

  assert.deepEqual(
    getCatalogPersonas(inactiveFirst).map((persona) => persona.id),
    getCatalogPersonas(activeFirst).map((persona) => persona.id),
  );
});

test("isCatalogPersonaSelected only treats active built-ins as selected", () => {
  assert.equal(
    isCatalogPersonaSelected(
      createPersona("builtin:reviewer", "Reviewer", {
        isBuiltIn: true,
        isActive: true,
      }),
    ),
    true,
  );
  assert.equal(
    isCatalogPersonaSelected(
      createPersona("builtin:solo", "Solo", {
        isBuiltIn: true,
        isActive: false,
      }),
    ),
    false,
  );
  assert.equal(
    isCatalogPersonaSelected(createPersona("custom:builder", "Builder")),
    false,
  );
});

test("getPersonaLabelsById keeps every returned persona addressable", () => {
  const personas = [
    createPersona("builtin:solo", "Solo", { isBuiltIn: true, isActive: false }),
    createPersona("custom:builder", "Builder"),
  ];

  assert.deepEqual(getPersonaLabelsById(personas), {
    "builtin:solo": "Solo",
    "custom:builder": "Builder",
  });
});

test("getPersonaLibraryState keeps the working library and full catalog in one place", () => {
  const personas = [
    createPersona("builtin:solo", "Solo", { isBuiltIn: true, isActive: false }),
    createPersona("builtin:kit", "Kit", {
      isBuiltIn: true,
      isActive: true,
    }),
    createPersona("custom:builder", "Builder"),
  ];

  const state = getPersonaLibraryState(personas);

  assert.deepEqual(
    state.libraryPersonas.map((persona) => persona.id),
    ["builtin:kit", "custom:builder"],
  );
  assert.deepEqual(
    state.catalogPersonas.map((persona) => persona.id),
    ["builtin:kit", "builtin:solo"],
  );
  assert.equal(state.personaLabelsById["builtin:solo"], "Solo");
});
