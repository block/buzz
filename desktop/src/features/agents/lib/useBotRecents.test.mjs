import assert from "node:assert/strict";
import test from "node:test";

import { pickQuickBotPersonas } from "./useBotRecents.ts";

function createPersona(id, displayName) {
  return {
    id,
    displayName,
    avatarUrl: null,
    systemPrompt: `${displayName} prompt`,
    runtime: null,
    model: null,
    isBuiltIn: true,
    isActive: true,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
  };
}

test("pickQuickBotPersonas prefers recents before defaults", () => {
  const personas = [
    createPersona("builtin:solo", "Solo"),
    createPersona("builtin:kit", "Kit"),
    createPersona("builtin:reviewer", "Reviewer"),
  ];

  assert.deepEqual(
    pickQuickBotPersonas(personas, ["builtin:reviewer"]).map(
      (persona) => persona.id,
    ),
    ["builtin:reviewer", "builtin:solo", "builtin:kit"],
  );
});

test("pickQuickBotPersonas falls back to any active personas when defaults are missing", () => {
  const personas = [
    createPersona("builtin:reviewer", "Reviewer"),
    createPersona("custom:planner", "Planner"),
    createPersona("custom:writer", "Writer"),
  ];

  assert.deepEqual(
    pickQuickBotPersonas(personas, []).map((persona) => persona.id),
    ["builtin:reviewer", "custom:planner", "custom:writer"],
  );
});

test("pickQuickBotPersonas skips duplicate and missing recents", () => {
  const personas = [
    createPersona("builtin:solo", "Solo"),
    createPersona("builtin:kit", "Kit"),
    createPersona("builtin:scout", "Scout"),
  ];

  assert.deepEqual(
    pickQuickBotPersonas(personas, [
      "builtin:solo",
      "missing",
      "builtin:solo",
      "builtin:kit",
    ]).map((persona) => persona.id),
    ["builtin:solo", "builtin:kit", "builtin:scout"],
  );
});
