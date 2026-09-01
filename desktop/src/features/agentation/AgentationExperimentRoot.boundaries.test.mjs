import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const source = await readFile(
  new URL("./AgentationExperimentRoot.tsx", import.meta.url),
  "utf8",
);

test("scope changes remount the stateful submission owner, not only Agentation", () => {
  assert.match(
    source,
    /<ScopedAgentationExperimentRoot\s+key=\{storageScope\}/,
  );
  const scopedOwner = source.indexOf("function ScopedAgentationExperimentRoot");
  for (const stateName of [
    "inFlight",
    "batchSubmission",
    "setStatus",
    "setAnnotations",
  ]) {
    assert.ok(
      source.indexOf(stateName) > scopedOwner,
      `${stateName} must be scope-owned`,
    );
  }
});

test("retained ambiguous event owns retry destination and accepted annotation snapshots", () => {
  assert.match(source, /const retainedSubmission = batchSubmission\.current/);
  assert.match(source, /submitDestination = retainedSubmission/);
  assert.match(source, /retained\?\.event\.content \?\?/);
  assert.match(source, /acceptedAnnotations,/);
  assert.match(source, /retained\?\.annotations \?\? batch/);
  assert.match(
    source,
    /JSON\.stringify\(accepted\) === JSON\.stringify\(annotation\)/,
  );
});
