import assert from "node:assert/strict";
import test from "node:test";

import { buildUnifiedGroups } from "./unifiedAgentGroups.ts";

const persona = {
  id: "debug-definition",
  displayName: "Debug",
  isActive: true,
};

test("Codex task agents stay in Custom agents after persona backfill", () => {
  const result = buildUnifiedGroups(
    [persona],
    [
      {
        name: "Debug",
        personaId: "debug-definition",
        codexTaskBinding: { taskId: "task-1" },
      },
    ],
  );

  assert.equal(result.groups[0].agents.length, 0);
  assert.equal(result.ungrouped.length, 1);
  assert.equal(result.ungrouped[0].name, "Debug");
});

test("ordinary persona-linked agents remain grouped", () => {
  const result = buildUnifiedGroups(
    [persona],
    [{ name: "Debug", personaId: "debug-definition" }],
  );

  assert.equal(result.groups[0].agents.length, 1);
  assert.equal(result.ungrouped.length, 0);
});
