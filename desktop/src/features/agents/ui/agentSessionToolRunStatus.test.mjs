import assert from "node:assert/strict";
import test from "node:test";

import {
  getToolRunGroupStatus,
  isToolRunGroupActive,
} from "./agentSessionToolRunStatus.ts";

const timestamp = "2026-06-14T22:20:23.000Z";

function tool(id, status, overrides = {}) {
  return {
    id,
    type: "tool",
    renderClass: "shell",
    descriptor: { renderClass: "shell", label: "Ran command", preview: null },
    title: "Ran command",
    toolName: "shell",
    buzzToolName: null,
    status,
    args: { command: "pnpm test" },
    result: "",
    isError: false,
    timestamp,
    startedAt: timestamp,
    completedAt: status === "completed" ? timestamp : null,
    turnId: "turn-1",
    ...overrides,
  };
}

test("an all-completed group reports completed", () => {
  assert.equal(
    getToolRunGroupStatus([tool("a", "completed"), tool("b", "completed")]),
    "completed",
  );
});

test("failure outranks completed, running, and pending", () => {
  const items = [
    tool("a", "completed"),
    tool("b", "executing"),
    tool("c", "pending"),
    tool("d", "failed"),
  ];
  assert.equal(getToolRunGroupStatus(items), "failed");
});

test("isError alone is enough to fail the group", () => {
  const items = [
    tool("a", "completed"),
    tool("b", "completed", { isError: true }),
  ];
  assert.equal(getToolRunGroupStatus(items), "failed");
});

test("running outranks pending and completed", () => {
  const items = [
    tool("a", "completed"),
    tool("b", "pending"),
    tool("c", "executing"),
  ];
  assert.equal(getToolRunGroupStatus(items), "executing");
});

test("pending outranks completed", () => {
  assert.equal(
    getToolRunGroupStatus([tool("a", "completed"), tool("b", "pending")]),
    "pending",
  );
});

test("status precedence is order-independent", () => {
  const failing = tool("a", "failed");
  const running = tool("b", "executing");
  const done = tool("c", "completed");
  assert.equal(getToolRunGroupStatus([failing, running, done]), "failed");
  assert.equal(getToolRunGroupStatus([done, running, failing]), "failed");
  assert.equal(getToolRunGroupStatus([running, failing, done]), "failed");
});

test("a lifecycle error row fails the group it sits in", () => {
  const items = [
    tool("a", "completed"),
    {
      id: "err",
      type: "lifecycle",
      renderClass: "error",
      title: "Session error",
      text: "boom",
      timestamp,
    },
  ];
  assert.equal(getToolRunGroupStatus(items), "failed");
});

test("an empty group has nothing outstanding", () => {
  assert.equal(getToolRunGroupStatus([]), "completed");
});

test("only executing and pending count as active", () => {
  assert.equal(isToolRunGroupActive("executing"), true);
  assert.equal(isToolRunGroupActive("pending"), true);
  assert.equal(isToolRunGroupActive("failed"), false);
  assert.equal(isToolRunGroupActive("completed"), false);
});
