import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";

test("retains the accessible custom issue-status control contract", async () => {
  const source = await readFile(
    new URL("./ProjectIssuesPanel.tsx", import.meta.url),
    "utf8",
  );

  assert.match(source, /aria-label="Change issue status"/);
  assert.match(source, /aria-label="Workflow status reason"/);
  assert.match(source, /border-input bg-background/);
  assert.match(source, /A reason is required for this workflow status\./);
  assert.match(source, /void handleSelect\(event\.target\.value/);
  assert.doesNotMatch(source, />\s*Set workflow status\s*</);
  assert.doesNotMatch(source, /signProjectIssueStatus/);
});
