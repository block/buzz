import assert from "node:assert/strict";
import { test } from "node:test";

import { projectFileTreeSummary } from "./projectFileTreeSummary.ts";

test("reports a complete repository tree normally", () => {
  assert.deepEqual(projectFileTreeSummary(12, 12), {
    countLabel: "12 files",
    truncationNotice: null,
  });
});

test("reports the loaded and total counts when the repository tree is truncated", () => {
  assert.deepEqual(projectFileTreeSummary(250, 661), {
    countLabel: "250 of 661 files",
    truncationNotice:
      "Showing the first 250 of 661 files. Some files and folders are not included.",
  });
});

test("never reports fewer total files than were loaded", () => {
  assert.deepEqual(projectFileTreeSummary(3, 0), {
    countLabel: "3 files",
    truncationNotice: null,
  });
});
