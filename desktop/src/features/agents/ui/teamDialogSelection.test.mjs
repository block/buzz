import assert from "node:assert/strict";
import test from "node:test";

import {
  copySelectedPersonaIds,
  countMissingPersonaIds,
  filterAvailablePersonaIds,
  orderPersonasByInitiallySelected,
} from "./teamDialogSelection.ts";

function createPersona(id) {
  return {
    id,
    displayName: id,
    avatarUrl: null,
    systemPrompt: "Prompt",
    runtime: null,
    model: null,
    namePool: [],
    isBuiltIn: false,
    isActive: true,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
  };
}

test("copySelectedPersonaIds keeps initial team members until submit filtering", () => {
  const selectedPersonaIds = copySelectedPersonaIds([
    "persona:available",
    "persona:late",
  ]);
  const initiallyAvailable = [createPersona("persona:available")];
  const laterAvailable = [
    createPersona("persona:available"),
    createPersona("persona:late"),
  ];

  assert.deepEqual(selectedPersonaIds, ["persona:available", "persona:late"]);
  assert.deepEqual(
    filterAvailablePersonaIds(selectedPersonaIds, initiallyAvailable),
    ["persona:available"],
  );
  assert.deepEqual(
    filterAvailablePersonaIds(selectedPersonaIds, laterAvailable),
    ["persona:available", "persona:late"],
  );
});

test("countMissingPersonaIds reports unresolved team personas", () => {
  assert.equal(
    countMissingPersonaIds(
      ["persona:available", "persona:missing-a", "persona:missing-b"],
      [createPersona("persona:available")],
    ),
    2,
  );
});

test("filterAvailablePersonaIds drops missing personas at submit time", () => {
  assert.deepEqual(
    filterAvailablePersonaIds(
      ["persona:missing", "persona:available"],
      [createPersona("persona:available")],
    ),
    ["persona:available"],
  );
});

test("orderPersonasByInitiallySelected keeps initially selected personas at top", () => {
  const personas = [
    createPersona("persona:michelangelo"),
    createPersona("persona:milhouse"),
    createPersona("persona:ned"),
    createPersona("persona:raphael"),
  ];

  const ordered = orderPersonasByInitiallySelected(personas, [
    "persona:milhouse",
    "persona:ned",
    "persona:missing",
  ]);

  assert.deepEqual(
    ordered.map((persona) => persona.id),
    [
      "persona:milhouse",
      "persona:ned",
      "persona:michelangelo",
      "persona:raphael",
    ],
  );
});
