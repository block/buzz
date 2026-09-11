import assert from "node:assert/strict";
import test from "node:test";

import {
  collectToolRunArtifacts,
  isLowSignalToolStep,
  partitionToolRunSteps,
} from "./agentSessionToolRunPartition.ts";

const timestamp = "2026-06-14T22:20:23.000Z";

function tool(id, renderClass, args, overrides = {}) {
  return {
    id,
    type: "tool",
    renderClass,
    descriptor: { renderClass, label: "Did a thing", preview: null },
    title: "Did a thing",
    toolName: renderClass,
    buzzToolName: null,
    status: "completed",
    args,
    result: "",
    isError: false,
    timestamp,
    startedAt: timestamp,
    completedAt: timestamp,
    turnId: "turn-1",
    ...overrides,
  };
}

const shell = (id, command, overrides) =>
  tool(id, "shell", { command }, overrides);

test("routine shell scaffolding is low signal", () => {
  assert.equal(isLowSignalToolStep(shell("a", "ls -la src")), true);
  assert.equal(isLowSignalToolStep(shell("b", "rg -n foo")), true);
  assert.equal(isLowSignalToolStep(shell("c", "wc -l file.ts")), true);
});

test("a pipeline is plumbing even when its head is meaningful", () => {
  assert.equal(
    isLowSignalToolStep(shell("a", "pnpm test 2>&1 | tail -5")),
    true,
  );
});

test("meaningful commands are never hidden", () => {
  assert.equal(isLowSignalToolStep(shell("a", "pnpm test")), false);
  assert.equal(isLowSignalToolStep(shell("b", "git commit")), false);
});

test("a failing or unfinished step is never hidden", () => {
  assert.equal(isLowSignalToolStep(shell("a", "ls", { isError: true })), false);
  assert.equal(
    isLowSignalToolStep(shell("b", "ls", { status: "failed" })),
    false,
  );
  assert.equal(
    isLowSignalToolStep(shell("c", "ls", { status: "executing" })),
    false,
  );
});

test("non-shell work is never hidden", () => {
  assert.equal(
    isLowSignalToolStep(tool("a", "file-edit", { path: "a.ts" })),
    false,
  );
  assert.equal(
    isLowSignalToolStep(tool("b", "relay-op", { channel: "c1" })),
    false,
  );
});

test("a short group hides nothing", () => {
  const items = [shell("a", "ls"), shell("b", "pwd"), shell("c", "wc -l x")];
  const { primaryItems, hiddenItems } = partitionToolRunSteps(items);
  assert.equal(hiddenItems.length, 0);
  assert.equal(primaryItems.length, 3);
});

test("a long group folds its scaffolding away", () => {
  const items = [
    shell("a", "ls"),
    shell("b", "pwd"),
    shell("c", "pnpm test"),
    tool("d", "file-edit", { path: "a.ts" }),
  ];
  const { primaryItems, hiddenItems } = partitionToolRunSteps(items);
  assert.deepEqual(
    hiddenItems.map((item) => item.id),
    ["a", "b"],
  );
  assert.deepEqual(
    primaryItems.map((item) => item.id),
    ["c", "d"],
  );
});

test("a group with only one hideable step keeps everything visible", () => {
  const items = [
    shell("a", "ls"),
    shell("b", "pnpm test"),
    tool("c", "file-edit", { path: "a.ts" }),
    tool("d", "file-edit", { path: "b.ts" }),
  ];
  assert.equal(partitionToolRunSteps(items).hiddenItems.length, 0);
});

test("a group that would fold to nothing keeps everything visible", () => {
  const items = [
    shell("a", "ls"),
    shell("b", "pwd"),
    shell("c", "cat x"),
    shell("d", "wc -l y"),
  ];
  const { primaryItems, hiddenItems } = partitionToolRunSteps(items);
  assert.equal(hiddenItems.length, 0);
  assert.equal(primaryItems.length, 4);
});

test("a step the reader opened is pinned visible", () => {
  const items = [
    shell("a", "ls"),
    shell("b", "pwd"),
    shell("c", "pnpm test"),
    tool("d", "file-edit", { path: "a.ts" }),
  ];
  const { primaryItems, hiddenItems } = partitionToolRunSteps(items, ["a"]);
  assert.equal(hiddenItems.length, 0);
  assert.equal(primaryItems.length, 4);
});

test("artifacts collect one entry per distinct file", () => {
  const artifacts = collectToolRunArtifacts([
    tool("a", "file-read", { path: "src/a.ts" }),
    tool("b", "file-edit", { path: "src/b.ts" }),
    tool("c", "file-read", { path: "src/a.ts" }),
    shell("d", "pnpm test"),
  ]);
  assert.deepEqual(
    artifacts.map((artifact) => [artifact.filename, artifact.kind]),
    [
      ["a.ts", "read"],
      ["b.ts", "edit"],
    ],
  );
});

test("an edit outranks a read of the same file", () => {
  const artifacts = collectToolRunArtifacts([
    tool("a", "file-read", { path: "src/a.ts" }),
    tool("b", "file-edit", { path: "src/a.ts" }),
  ]);
  assert.equal(artifacts.length, 1);
  assert.equal(artifacts[0].kind, "edit");
  assert.equal(artifacts[0].itemId, "b");
});

test("a failed step contributes no artifact", () => {
  assert.deepEqual(
    collectToolRunArtifacts([
      tool("a", "file-edit", { path: "src/a.ts" }, { isError: true }),
    ]),
    [],
  );
});
