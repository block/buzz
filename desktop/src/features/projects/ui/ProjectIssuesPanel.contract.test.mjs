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
  assert.doesNotMatch(source, /signProjectIssueStatus/);
});
