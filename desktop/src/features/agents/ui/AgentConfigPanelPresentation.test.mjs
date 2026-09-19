import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const source = await readFile(
  new URL("./AgentConfigPanel.tsx", import.meta.url),
  "utf8",
);

test("shared configuration rows show only the effective value", () => {
  for (const forbiddenPattern of [
    /Available after agent starts/,
    /provenanceSentence/,
    /ProvenanceHint/,
    /field\.overriddenValue/,
    /line-through/,
    /isPreSpawn\s*&&\s*["']opacity-/,
  ]) {
    assert.doesNotMatch(source, forbiddenPattern);
  }
});

test("unknown normalized values use an em dash", () => {
  assert.match(source, /const rawDisplayValue = field\.value \?\? "—";/);
});

test("profile model rows keep their bare leading icons", () => {
  assert.match(source, /data-slot="agent-config-field-icon"/);
  assert.doesNotMatch(source, /rounded-full bg-muted[^\n]*<Icon/);
});
