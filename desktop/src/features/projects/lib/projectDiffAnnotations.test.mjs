import assert from "node:assert/strict";
import { test } from "node:test";

import {
  annotationToBuzzAnchor,
  buildFileDiffAnnotations,
  buzzSideToDiffSide,
  diffSideToBuzzSide,
  focusedAnchorToSelectedRange,
  groupCommentsByAnchor,
  isRenderablePatch,
  patchBodyLineCount,
} from "./projectDiffAnnotations.ts";

const OLD_ANCHOR = { line: 3, path: "src/a.ts", side: "old" };
const NEW_ANCHOR = { line: 5, path: "src/a.ts", side: "new" };

function comment(id, anchor) {
  return {
    id,
    anchor,
    createdAt: 1,
    author: "author",
    content: "note",
    tags: [],
  };
}

test("maps Buzz old/new anchor sides to Pierre deletions/additions and back", () => {
  assert.equal(buzzSideToDiffSide("old"), "deletions");
  assert.equal(buzzSideToDiffSide("new"), "additions");
  assert.equal(diffSideToBuzzSide("deletions"), "old");
  assert.equal(diffSideToBuzzSide("additions"), "new");
});

test("groups comments by path/side/line and excludes other files", () => {
  const comments = [
    comment("1", OLD_ANCHOR),
    comment("2", OLD_ANCHOR),
    comment("3", NEW_ANCHOR),
    comment("4", { line: 9, path: "src/other.ts", side: "new" }),
    comment("5", { line: 3, path: "src/a.ts", side: "new" }),
  ];
  const groups = groupCommentsByAnchor("src/a.ts", comments);
  assert.deepEqual(
    [...groups.keys()].sort(),
    ["new:3", "old:3", "new:5"].sort(),
  );
  assert.equal(groups.get("old:3")?.length, 2);
  assert.equal(groups.get("new:5")?.length, 1);
  assert.equal(groups.get("new:3")?.length, 1);
});

test("builds annotations with exact anchor metadata and stable ordering", () => {
  // Same line 7: new-side comment supplied first, old-side supplied second —
  // the deletion-before-addition tie-break must still win on equal lines.
  const SAME_LINE_NEW = { line: 7, path: "src/a.ts", side: "new" };
  const SAME_LINE_OLD = { line: 7, path: "src/a.ts", side: "old" };
  const comments = [
    comment("1", NEW_ANCHOR),
    comment("2", SAME_LINE_NEW),
    comment("3", SAME_LINE_OLD),
    comment("4", OLD_ANCHOR),
  ];
  const annotations = buildFileDiffAnnotations("src/a.ts", comments, null);
  // Sorted by line ascending, then deletions before additions on equal lines.
  assert.deepEqual(
    annotations.map((a) => [a.side, a.lineNumber]),
    [
      ["deletions", 3],
      ["additions", 5],
      ["deletions", 7],
      ["additions", 7],
    ],
  );
  assert.equal(annotations[0].metadata.anchor.path, "src/a.ts");
  assert.equal(annotations[0].metadata.comments.length, 1);
  assert.equal(annotations[1].metadata.focused, false);
  // The same-line group keeps both comments, old side first despite the
  // reversed input order.
  assert.equal(annotations[2].metadata.anchor.side, "old");
  assert.equal(annotations[3].metadata.anchor.side, "new");
  assert.equal(annotations[2].metadata.comments.length, 1);
  assert.equal(annotations[3].metadata.comments.length, 1);
});

test("includes the focused anchor even with no comments and marks it focused", () => {
  const annotations = buildFileDiffAnnotations("src/a.ts", [], NEW_ANCHOR);
  assert.equal(annotations.length, 1);
  assert.equal(annotations[0].side, "additions");
  assert.equal(annotations[0].lineNumber, 5);
  assert.equal(annotations[0].metadata.focused, true);
  assert.deepEqual(annotations[0].metadata.comments, []);
});

test("does not add a focused annotation for another file", () => {
  const annotations = buildFileDiffAnnotations("src/a.ts", [], {
    line: 1,
    path: "src/b.ts",
    side: "new",
  });
  assert.equal(annotations.length, 0);
});

test("marks the focused flag on an existing comment group", () => {
  const comments = [comment("1", NEW_ANCHOR)];
  const annotations = buildFileDiffAnnotations(
    "src/a.ts",
    comments,
    NEW_ANCHOR,
  );
  assert.equal(annotations.length, 1);
  assert.equal(annotations[0].metadata.focused, true);
  assert.equal(annotations[0].metadata.comments.length, 1);
});

test("annotation metadata round-trips to the exact Buzz anchor", () => {
  const annotations = buildFileDiffAnnotations(
    "src/a.ts",
    [comment("1", OLD_ANCHOR)],
    null,
  );
  const recovered = annotationToBuzzAnchor(annotations[0]);
  assert.deepEqual(recovered, OLD_ANCHOR);
});

test("maps a focused anchor to a single-line Pierre selected range", () => {
  assert.deepEqual(focusedAnchorToSelectedRange(null), null);
  assert.deepEqual(focusedAnchorToSelectedRange(undefined), null);
  assert.deepEqual(focusedAnchorToSelectedRange(OLD_ANCHOR), {
    start: 3,
    end: 3,
    side: "deletions",
  });
  assert.deepEqual(focusedAnchorToSelectedRange(NEW_ANCHOR), {
    start: 5,
    end: 5,
    side: "additions",
  });
});

test("detects renderable patches (at least one valid hunk header)", () => {
  assert.equal(isRenderablePatch(""), false);
  assert.equal(isRenderablePatch("   \n\n"), false);
  assert.equal(isRenderablePatch("this is not a patch at all"), false);
  assert.equal(
    isRenderablePatch(
      "diff --git a/x b/x\nindex 1..2 100644\nBinary files differ\n",
    ),
    false,
  );
  // A fake `@@ ` line without numeric old/new ranges is not a hunk header.
  assert.equal(isRenderablePatch("@@ not a hunk header\n"), false);
  assert.equal(
    isRenderablePatch("@@ -abc +def @@\nno numeric ranges\n"),
    false,
  );
  assert.equal(isRenderablePatch("@@ -1,2 +1,2 @@\n-old\n+new\n"), true);
  // Hunk headers may omit the line counts on either side.
  assert.equal(isRenderablePatch("@@ -1 +1 @@\n-old\n+new\n"), true);
  assert.equal(isRenderablePatch("@@ -1 +2,3 @@\n context\n"), true);
  // A trailing section/function label after the closing @@ is valid.
  assert.equal(
    isRenderablePatch("@@ -12,3 +12,4 @@ function renderDiff()\n-old\n+new\n"),
    true,
  );
});

test("counts patch body lines like the previous renderer", () => {
  const patch = [
    "diff --git a/a.ts b/a.ts",
    "index 1..2 100644",
    "--- a/a.ts",
    "+++ b/a.ts",
    "@@ -1,2 +1,2 @@",
    "-old",
    "+new",
    " context",
  ].join("\n");
  assert.equal(patchBodyLineCount(patch), 4);
  assert.equal(patchBodyLineCount(""), 0);
});
