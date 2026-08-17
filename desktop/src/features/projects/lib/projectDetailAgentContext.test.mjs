import assert from "node:assert/strict";
import test from "node:test";

import {
  buildProjectDetailAgentContext,
  projectDetailAgentContextBlock,
  stripProjectDetailAgentContext,
} from "./projectDetailAgentContext.ts";

const base = {
  activeTab: "files",
  branch: "main",
  file: { kind: "file", path: "src/app.tsx" },
  project: { name: "Buzz Patrol" },
  repository: { name: "Buzz", repoAddress: "owner:buzz" },
  source: "local",
  workItems: [null, null, null],
};

test("builds selected file context", () => {
  const context = buildProjectDetailAgentContext(base);
  assert.equal(context.view, "Files");
  assert.deepEqual(context.file, { kind: "file", path: "src/app.tsx" });
  assert.equal(context.workItem, null);
});

test("review detail takes precedence over its workspace tab", () => {
  const context = buildProjectDetailAgentContext({
    ...base,
    activeTab: "prs",
    workItems: [
      null,
      null,
      { id: "review-42", status: "Open", title: "Ship the fix" },
    ],
  });
  assert.equal(context.view, "Review detail");
  assert.deepEqual(context.workItem, {
    id: "review-42",
    kind: "review",
    status: "Open",
    title: "Ship the fix",
  });
  assert.equal(context.file, null);
});

test("prompt footer contains current page details", () => {
  const footer = projectDetailAgentContextBlock(
    buildProjectDetailAgentContext(base),
  );
  assert.match(footer, /Current Buzz project page:/);
  assert.match(footer, /Repository: Buzz \(owner:buzz\)/);
  assert.match(footer, /View: Files/);
  assert.match(footer, /File: src\/app\.tsx/);
  assert.match(footer, /Branch: main/);
});

test("strips hidden page context from the displayed user message", () => {
  const content = `Explain this file${projectDetailAgentContextBlock(
    buildProjectDetailAgentContext(base),
  )}`;
  assert.equal(stripProjectDetailAgentContext(content), "Explain this file");
});
