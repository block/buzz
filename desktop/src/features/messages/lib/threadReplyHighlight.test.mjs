import assert from "node:assert/strict";
import { test } from "node:test";
import { selectThreadRowHighlight } from "./threadReplyHighlight.ts";

// The hovered branch spans rows in the half-open range (startIndex, endIndex].
const branch = { id: "b", depth: 1, startIndex: 2, endIndex: 5 };

test("row-highlight: null branch highlights nothing", () => {
  assert.deepEqual(
    selectThreadRowHighlight({
      branch: null,
      index: 3,
      messageId: "x",
      messageDepth: 2,
      showGuides: true,
    }),
    {
      isBranchOwner: false,
      isInsideBranch: false,
      isDirectChild: false,
      lineDepths: undefined,
    },
  );
});

test("row-highlight: the branch owner is flagged but is not inside its own range", () => {
  const h = selectThreadRowHighlight({
    branch,
    index: 2,
    messageId: "b",
    messageDepth: 1,
    showGuides: true,
  });
  assert.equal(h.isBranchOwner, true);
  // startIndex is excluded, so the owner row itself is not "inside".
  assert.equal(h.isInsideBranch, false);
  assert.equal(h.lineDepths, undefined);
});

test("row-highlight: a direct child inside the branch draws the guide line", () => {
  const h = selectThreadRowHighlight({
    branch,
    index: 3,
    messageId: "c",
    messageDepth: 2,
    showGuides: true,
  });
  assert.equal(h.isInsideBranch, true);
  assert.equal(h.isDirectChild, true);
  assert.deepEqual(h.lineDepths, [1]);
});

test("row-highlight: a deeper descendant is inside but not a direct child", () => {
  const h = selectThreadRowHighlight({
    branch,
    index: 4,
    messageId: "d",
    messageDepth: 3,
    showGuides: true,
  });
  assert.equal(h.isInsideBranch, true);
  assert.equal(h.isDirectChild, false);
});

test("row-highlight: a row past endIndex is outside the branch", () => {
  const h = selectThreadRowHighlight({
    branch,
    index: 6,
    messageId: "e",
    messageDepth: 2,
    showGuides: true,
  });
  assert.equal(h.isInsideBranch, false);
});

test("row-highlight: guides suppressed → no line depths even inside the branch", () => {
  const h = selectThreadRowHighlight({
    branch,
    index: 3,
    messageId: "c",
    messageDepth: 2,
    showGuides: false,
  });
  assert.equal(h.isInsideBranch, true);
  assert.equal(h.lineDepths, undefined);
});
