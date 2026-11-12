import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const source = readFileSync(
  new URL("./ProjectHomeWorkspaceSheet.tsx", import.meta.url),
  "utf8",
);

test("aggregated commit detail uses the repository that owns the selected commit", () => {
  const detailPanel = source.match(/<ProjectCommitDetailPanel[\s\S]*?\/>/)?.[0];

  assert.ok(detailPanel, "expected the commit detail panel to be rendered");
  assert.match(detailPanel, /project=\{selectedCommitRepository\}/);
});
