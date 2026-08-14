import assert from "node:assert/strict";
import { test } from "node:test";

import { JSDOM } from "jsdom";

import {
  createFocusOneShotState,
  createHoverAnchorCache,
  findAnnotationRow,
  focusAnnotationRow,
  focusedAnchorKey,
  includeActiveDiffAnchor,
  markFocusSucceeded,
  nextFocusAttempt,
  recordHoveredLine,
  resolveGutterAnchor,
  selectedRangeForFile,
} from "./projectDiffFocus.ts";
import { buildFileDiffAnnotations } from "./projectDiffAnnotations.ts";

const dom = new JSDOM("<!doctype html><html><body></body></html>");
const { document } = dom.window;

/**
 * Build a minimal `diffs-container` with an open shadow root and a
 * `[data-line-annotation]` row containing the documented annotation slot
 * (named by `getLineAnnotationName`: `annotation-{side}-{lineNumber}`).
 */
function containerWithAnnotation(side, lineNumber) {
  const container = document.createElement("diffs-container");
  const shadowRoot = container.attachShadow({ mode: "open" });
  const row = document.createElement("div");
  row.setAttribute("data-line-annotation", "0,0");
  const content = document.createElement("div");
  content.setAttribute("data-annotation-content", "");
  const slot = document.createElement("slot");
  slot.name = `annotation-${side}-${lineNumber}`;
  content.appendChild(slot);
  row.appendChild(content);
  shadowRoot.appendChild(row);
  return { container, row, slot };
}

test("finds the annotation row for additions and deletions slots", () => {
  const { container, row } = containerWithAnnotation("additions", 5);
  assert.equal(findAnnotationRow(container, "additions", 5), row);
});

test("returns null when the slot does not exist for that side/line", () => {
  const { container } = containerWithAnnotation("additions", 5);
  assert.equal(findAnnotationRow(container, "deletions", 5), null);
  assert.equal(findAnnotationRow(container, "additions", 6), null);
});

test("returns null when the container has no shadow root", () => {
  const plain = document.createElement("div");
  assert.equal(findAnnotationRow(plain, "additions", 5), null);
});

test("focuses and scrolls the annotation row into view", () => {
  const { container, row } = containerWithAnnotation("deletions", 3);
  // Attach so focus() can resolve a real active element in jsdom.
  document.body.appendChild(container);
  let scrolled = false;
  row.scrollIntoView = () => {
    scrolled = true;
  };
  const focused = focusAnnotationRow(container, "deletions", 3);
  assert.equal(focused, true);
  assert.equal(scrolled, true);
  assert.equal(row.getAttribute("tabindex"), "-1");
  // Focus inside a shadow root is reported by the shadow root's
  // activeElement, not the top document's.
  assert.equal(container.shadowRoot.activeElement, row);
  container.remove();
});

test("focus returns false when the row cannot be resolved", () => {
  const { container } = containerWithAnnotation("additions", 5);
  assert.equal(focusAnnotationRow(container, "deletions", 9), false);
});

// --- Active-anchor annotation merging (review regression coverage) ---

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

const PATH = "src/a.ts";
const NEW_5 = { line: 5, path: PATH, side: "new" };
const OLD_3 = { line: 3, path: PATH, side: "old" };

test("a commentless active anchor produces an annotation slot", () => {
  const annotations = includeActiveDiffAnchor([], PATH, NEW_5);
  assert.equal(annotations.length, 1);
  assert.equal(annotations[0].side, "additions");
  assert.equal(annotations[0].lineNumber, 5);
  assert.deepEqual(annotations[0].metadata.comments, []);
  assert.equal(annotations[0].metadata.focused, false);
  assert.deepEqual(annotations[0].metadata.anchor, NEW_5);
});

test("active and focused anchors coexist with stable ordering", () => {
  const base = buildFileDiffAnnotations(PATH, [comment("1", OLD_3)], NEW_5);
  // base: old:3 (comments) + new:5 (focused)
  const annotations = includeActiveDiffAnchor(base, PATH, NEW_5);
  // active anchor equals focused anchor — no duplicate.
  assert.equal(annotations.length, 2);
  assert.deepEqual(
    annotations.map((a) => [a.side, a.lineNumber]),
    [
      ["deletions", 3],
      ["additions", 5],
    ],
  );
});

test("a distinct active anchor is added and ordering is stable", () => {
  const base = buildFileDiffAnnotations(PATH, [], NEW_5);
  const distinctActive = { line: 9, path: PATH, side: "old" };
  const annotations = includeActiveDiffAnchor(base, PATH, distinctActive);
  // Line ascending: additions:5 before deletions:9 (tie-break only on equal lines).
  assert.deepEqual(
    annotations.map((a) => [a.side, a.lineNumber]),
    [
      ["additions", 5],
      ["deletions", 9],
    ],
  );
  const active = annotations.find((a) => a.lineNumber === 9);
  assert.equal(active.metadata.focused, false);
  assert.deepEqual(active.metadata.anchor, distinctActive);
});

test("an active anchor matching an existing comment group is deduplicated", () => {
  const base = buildFileDiffAnnotations(PATH, [comment("1", OLD_3)], null);
  assert.equal(base.length, 1);
  const annotations = includeActiveDiffAnchor(base, PATH, OLD_3);
  assert.equal(annotations.length, 1);
  assert.equal(annotations[0].metadata.comments.length, 1);
});

test("an active anchor from another file is ignored", () => {
  const annotations = includeActiveDiffAnchor([], PATH, {
    line: 1,
    path: "src/other.ts",
    side: "new",
  });
  assert.equal(annotations.length, 0);
});

// --- File-scoped focus state (review regression coverage) ---

test("selected range is scoped to the rendered file path", () => {
  assert.equal(selectedRangeForFile(PATH, null), null);
  assert.equal(
    selectedRangeForFile(PATH, { line: 1, path: "src/b.ts", side: "new" }),
    null,
  );
  assert.deepEqual(selectedRangeForFile(PATH, NEW_5), {
    start: 5,
    end: 5,
    side: "additions",
  });
});

test("focus key includes the path so same side/line on a new file is a new target", () => {
  const samePositionOtherPath = { line: 5, path: "src/b.ts", side: "new" };
  assert.equal(focusedAnchorKey(PATH, NEW_5), "src/a.ts:new:5");
  assert.equal(focusedAnchorKey(PATH, samePositionOtherPath), null);
  assert.equal(focusedAnchorKey(PATH, null), null);
  assert.equal(
    focusedAnchorKey("src/b.ts", samePositionOtherPath),
    "src/b.ts:new:5",
  );
});

// --- One-shot focused-line lifecycle (review regression coverage) ---

test("a null key clears the remembered focus", () => {
  let state = createFocusOneShotState();
  state = markFocusSucceeded("src/a.ts:new:5");
  assert.equal(state.lastFocusedKey, "src/a.ts:new:5");

  const cleared = nextFocusAttempt(state, null);
  assert.equal(cleared.attempt, false);
  assert.equal(cleared.state.lastFocusedKey, null);
});

test("a successfully focused key is skipped on later passes (one-shot)", () => {
  let state = createFocusOneShotState();
  const first = nextFocusAttempt(state, "src/a.ts:new:5");
  assert.equal(first.attempt, true);
  state = markFocusSucceeded("src/a.ts:new:5");

  const again = nextFocusAttempt(state, "src/a.ts:new:5");
  assert.equal(again.attempt, false);
  assert.equal(again.state.lastFocusedKey, "src/a.ts:new:5");
});

test("reset then reselect the same anchor is a fresh focus target", () => {
  let state = createFocusOneShotState();
  // Focus succeeds on the anchor.
  let next = nextFocusAttempt(state, "src/a.ts:new:5");
  assert.equal(next.attempt, true);
  state = markFocusSucceeded("src/a.ts:new:5");
  // Focus is cleared (null key).
  next = nextFocusAttempt(state, null);
  state = next.state;
  assert.equal(state.lastFocusedKey, null);
  // The same anchor selected again must be attempted, not skipped.
  next = nextFocusAttempt(state, "src/a.ts:new:5");
  assert.equal(next.attempt, true);
});

test("a failed focus stays retryable on a later pass", () => {
  let state = createFocusOneShotState();
  // First pass: key is new, attempt runs, but focusAnnotationRow fails —
  // the caller must NOT remember the key on failure.
  let next = nextFocusAttempt(state, "src/a.ts:new:5");
  assert.equal(next.attempt, true);
  state = next.state; // no markFocusSucceeded on failure
  assert.equal(state.lastFocusedKey, null);

  // A later public onPostRender update with the same key retries.
  next = nextFocusAttempt(state, "src/a.ts:new:5");
  assert.equal(next.attempt, true);
  // This time it succeeds.
  state = markFocusSucceeded("src/a.ts:new:5");
  // And is now one-shot.
  next = nextFocusAttempt(state, "src/a.ts:new:5");
  assert.equal(next.attempt, false);
});

test("a genuinely new key is never skipped after a different success", () => {
  let state = createFocusOneShotState();
  state = markFocusSucceeded("src/a.ts:new:5");
  const next = nextFocusAttempt(state, "src/a.ts:old:9");
  assert.equal(next.attempt, true);
});

// --- Hovered-line anchor cache (gutter fallback regression coverage) ---

test("cached onLineEnter anchor is the fallback when the live getter is absent", () => {
  let cache = createHoverAnchorCache();
  cache = recordHoveredLine(PATH, 5, "additions");
  const anchor = resolveGutterAnchor(cache, PATH, undefined);
  assert.deepEqual(anchor, { lineNumber: 5, side: "additions" });
});

test("the live getter always wins over a stale cache", () => {
  let cache = createHoverAnchorCache();
  cache = recordHoveredLine(PATH, 5, "additions");
  const anchor = resolveGutterAnchor(cache, PATH, {
    lineNumber: 9,
    side: "deletions",
  });
  assert.deepEqual(anchor, { lineNumber: 9, side: "deletions" });
});

test("no anchor when both the live getter and cache are absent", () => {
  const cache = createHoverAnchorCache();
  assert.equal(resolveGutterAnchor(cache, PATH, undefined), null);
  assert.equal(resolveGutterAnchor(cache, PATH, null), null);
});

test("a stale cached anchor from another file never survives", () => {
  let cache = createHoverAnchorCache();
  cache = recordHoveredLine("src/b.ts", 7, "deletions");
  assert.equal(resolveGutterAnchor(cache, PATH, undefined), null);
  // A later onLineEnter for the current file replaces the entry entirely.
  cache = recordHoveredLine(PATH, 3, "deletions");
  assert.deepEqual(resolveGutterAnchor(cache, PATH, undefined), {
    lineNumber: 3,
    side: "deletions",
  });
});

test("a later onLineEnter replaces the cached anchor", () => {
  let cache = createHoverAnchorCache();
  cache = recordHoveredLine(PATH, 5, "additions");
  cache = recordHoveredLine(PATH, 6, "additions");
  assert.deepEqual(resolveGutterAnchor(cache, PATH, undefined), {
    lineNumber: 6,
    side: "additions",
  });
});
