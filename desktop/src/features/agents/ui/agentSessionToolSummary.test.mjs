import assert from "node:assert/strict";
import test from "node:test";

import {
  buildCompactToolSummary,
  isCompactDeveloperTool,
} from "./agentSessionToolSummary.ts";

const baseTimestamp = "2026-06-14T19:00:00.000Z";

function makeTool(overrides = {}) {
  return {
    id: "tool:1",
    type: "tool",
    title: "Tool call",
    toolName: "shell",
    buzzToolName: null,
    status: "completed",
    args: {},
    result: "",
    isError: false,
    timestamp: baseTimestamp,
    startedAt: baseTimestamp,
    completedAt: "2026-06-14T19:00:01.000Z",
    ...overrides,
  };
}

test("isCompactDeveloperTool returns false for Buzz relay tools", () => {
  assert.equal(
    isCompactDeveloperTool(
      makeTool({
        toolName: "send_message",
        buzzToolName: "send_message",
        title: "Send Message",
      }),
    ),
    false,
  );
});

test("isCompactDeveloperTool detects buzz-dev-mcp shell tools", () => {
  assert.equal(
    isCompactDeveloperTool(
      makeTool({
        toolName: "buzz-dev-mcp__shell",
        title: "buzz-dev-mcp__shell",
      }),
    ),
    true,
  );
});

test("buildCompactToolSummary formats shell command preview", () => {
  const summary = buildCompactToolSummary(
    makeTool({
      toolName: "buzz-dev-mcp__shell",
      args: { command: "git status" },
    }),
  );

  assert.equal(summary.label, "Ran command");
  assert.equal(summary.preview, "git status");
});

test("buildCompactToolSummary formats view_image source preview", () => {
  const summary = buildCompactToolSummary(
    makeTool({
      toolName: "buzz-dev-mcp__view_image",
      args: {
        source:
          "https://sprout-oss.stage.blox.sqprod.co/media/ffd1b2721f2d52e19f0ca2be9aa7842cdec5b4e0215aaab2a67c26a2a76a6a83.png",
      },
    }),
  );

  assert.equal(summary.label, "Viewed image");
  assert.ok(summary.preview?.startsWith("https://sprout-oss"));
});

test("buildCompactToolSummary formats read_file path preview", () => {
  const summary = buildCompactToolSummary(
    makeTool({
      toolName: "read_file",
      args: { path: "desktop/src/app/App.tsx" },
    }),
  );

  assert.equal(summary.label, "Read file");
  assert.equal(summary.preview, "desktop/src/app/App.tsx");
});

test("buildCompactToolSummary formats todo list preview", () => {
  const summary = buildCompactToolSummary(
    makeTool({
      toolName: "todo",
      args: {
        todos: [
          { text: "Ship compact summaries", done: false },
          { text: "Verify UI", done: false },
        ],
      },
    }),
  );

  assert.equal(summary.label, "Updated todos");
  assert.equal(summary.preview, "Ship compact summaries (+1)");
});

test("buildCompactToolSummary uses running and failed labels", () => {
  assert.equal(
    buildCompactToolSummary(
      makeTool({ toolName: "str_replace", status: "executing" }),
    ).label,
    "Editing file",
  );
  assert.equal(
    buildCompactToolSummary(
      makeTool({ toolName: "str_replace", status: "failed", isError: true }),
    ).label,
    "Edit failed",
  );
});
