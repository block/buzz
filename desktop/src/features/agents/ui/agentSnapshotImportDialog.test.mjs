import assert from "node:assert/strict";
import test from "node:test";

// Source-path tests for AgentSnapshotImportDialog ResultBody rendering.
//
// ResultBody is a hook-free component: it accepts a result object and renders
// the summary, partial-memory alert, and per-entry error list. These tests
// call it as a plain function and walk the element tree to verify that
// memoryErrors strings are carried through to a bounded list element with the
// correct data-testid. No DOM or test renderer is needed.

import { ResultBody } from "./AgentSnapshotImportDialog.tsx";

/**
 * Walk a React element tree (breadth-first) and collect all elements that
 * match a predicate.
 */
function findAll(element, predicate) {
  if (!element || typeof element !== "object") return [];
  const matches = [];
  const queue = [element];
  while (queue.length > 0) {
    const node = queue.shift();
    if (!node || typeof node !== "object") continue;
    if (predicate(node)) matches.push(node);
    const children = node.props?.children;
    if (Array.isArray(children)) {
      queue.push(...children.flat(Infinity).filter(Boolean));
    } else if (children && typeof children === "object") {
      queue.push(children);
    }
  }
  return matches;
}

/**
 * Collect all string leaves in the element tree.
 */
function collectText(element) {
  const texts = [];
  const queue = [element];
  while (queue.length > 0) {
    const node = queue.shift();
    if (typeof node === "string") {
      texts.push(node);
      continue;
    }
    if (!node || typeof node !== "object") continue;
    const children = node.props?.children;
    if (Array.isArray(children)) {
      queue.push(...children.flat(Infinity).filter(Boolean));
    } else if (typeof children === "string") {
      texts.push(children);
    } else if (children && typeof children === "object") {
      queue.push(children);
    }
  }
  return texts;
}

function makeResult(overrides = {}) {
  return {
    displayName: "TestBot",
    newPubkey: "abc123",
    personaId: "persona-1",
    memoryWritten: 0,
    memoryTotal: 2,
    memoryErrors: [],
    profileSyncError: null,
    ...overrides,
  };
}

// ── memory errors detail list ─────────────────────────────────────────────────

test("result_body_renders_memory_errors_list_with_test_id", () => {
  const errors = [
    'slug "mem/notes": relay timeout',
    'slug "core": build failed: key mismatch',
  ];
  const result = makeResult({
    memoryWritten: 0,
    memoryTotal: 2,
    memoryErrors: errors,
  });

  const element = ResultBody({ result, confirmError: null });

  // The bounded list must carry the expected data-testid.
  const errLists = findAll(
    element,
    (n) => n.props?.["data-testid"] === "agent-snapshot-import-memory-errors",
  );
  assert.equal(
    errLists.length,
    1,
    "exactly one memory-errors list must be rendered",
  );

  // The list must be vertically bounded (max-h-*) and scrollable
  // (overflow-y-auto) so it cannot grow the dialog without bound.
  const listNode = errLists[0];
  const className = listNode.props?.className ?? "";
  assert.ok(
    /max-h-/.test(className),
    `memory-errors list must have a max-height class (got: "${className}")`,
  );
  assert.ok(
    /overflow-y-auto/.test(className),
    `memory-errors list must have overflow-y-auto (got: "${className}")`,
  );

  // Each item must use break-all (not truncate) so the full error text is
  // readable, not clipped.
  const items = findAll(element, (n) => n.type === "li");
  assert.ok(items.length > 0, "list items must be present");
  for (const item of items) {
    const cls = item.props?.className ?? "";
    assert.ok(
      /break-all/.test(cls),
      `list item must use break-all for full error visibility (got: "${cls}")`,
    );
    assert.ok(
      !/truncate/.test(cls),
      `list item must not use truncate (got: "${cls}")`,
    );
  }
});

test("result_body_surfaces_both_error_strings_in_tree", () => {
  const errors = [
    'slug "mem/notes": relay timeout',
    'slug "core": build failed: key mismatch',
  ];
  const result = makeResult({
    memoryWritten: 0,
    memoryTotal: 2,
    memoryErrors: errors,
  });

  const element = ResultBody({ result, confirmError: null });
  const allText = collectText(element).join(" ");

  assert.ok(
    allText.includes('slug "mem/notes"'),
    "first error slug must appear in the rendered tree",
  );
  assert.ok(
    allText.includes('slug "core"'),
    "second error slug must appear in the rendered tree",
  );
});

test("result_body_full_success_omits_memory_errors_list", () => {
  const result = makeResult({
    memoryWritten: 2,
    memoryTotal: 2,
    memoryErrors: [],
  });

  const element = ResultBody({ result, confirmError: null });

  const errLists = findAll(
    element,
    (n) => n.props?.["data-testid"] === "agent-snapshot-import-memory-errors",
  );
  assert.equal(
    errLists.length,
    0,
    "full-success result must not render the memory-errors list",
  );
});
