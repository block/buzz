import assert from "node:assert/strict";
import test from "node:test";

import { buildCompactToolSummary } from "./agentSessionToolSummary.ts";

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

test("buildCompactToolSummary formats Buzz send_message preview", () => {
  const summary = buildCompactToolSummary(
    makeTool({
      toolName: "send_message",
      buzzToolName: "send_message",
      title: "Send Message",
      args: { content: "Hello team" },
    }),
  );

  assert.equal(summary.kind, "message");
  assert.equal(summary.label, "Send Message");
  assert.equal(summary.preview, "Hello team");
  assert.equal(summary.presentation, "message");
});

test("buildCompactToolSummary treats buzz messages send commands as messages", () => {
  const summary = buildCompactToolSummary(
    makeTool({
      toolName: "buzz-dev-mcp__shell",
      args: {
        command:
          'buzz --format compact messages send --channel channel-1 --content "@Ned are you working"',
      },
    }),
  );

  assert.equal(summary.kind, "message");
  assert.equal(summary.label, "Send Message");
  assert.equal(summary.preview, "@Ned are you working");
  assert.equal(summary.presentation, "message");
});

test("buildCompactToolSummary extracts simple piped buzz message content", () => {
  const summary = buildCompactToolSummary(
    makeTool({
      toolName: "shell",
      args: {
        command:
          'echo "hello from stdin" | ./target/release/buzz messages send --channel channel-1 --content -',
      },
    }),
  );

  assert.equal(summary.label, "Send Message");
  assert.equal(summary.preview, "hello from stdin");
  assert.equal(summary.presentation, "message");
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
  assert.equal(summary.presentation, "inline");
});

test("buildCompactToolSummary formats view_image thumbnail source", () => {
  const source =
    "https://sprout-oss.stage.blox.sqprod.co/media/ffd1b2721f2d52e19f0ca2be9aa7842cdec5b4e0215aaab2a67c26a2a76a6a83.png";
  const summary = buildCompactToolSummary(
    makeTool({
      toolName: "buzz-dev-mcp__view_image",
      args: { source },
    }),
  );

  assert.equal(summary.label, "Viewed image");
  assert.equal(summary.thumbnailSrc, source);
  assert.equal(summary.preview, source);
});

test("buildCompactToolSummary uses basename for local view_image paths", () => {
  const summary = buildCompactToolSummary(
    makeTool({
      toolName: "view_image",
      args: { source: "desktop/assets/screenshot.png" },
    }),
  );

  assert.equal(summary.thumbnailSrc, null);
  assert.equal(summary.preview, "screenshot.png");
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

test("buildCompactToolSummary promotes non-send buzz CLI commands to relay ops", () => {
  const summary = buildCompactToolSummary(
    makeTool({
      toolName: "shell",
      args: {
        command: "buzz channels get --channel channel-1",
      },
    }),
  );

  assert.equal(summary.kind, "relay-op");
  assert.equal(summary.label, "Channels Get");
  assert.equal(summary.preview, "channel-1");
  assert.equal(summary.presentation, "inline");
});

test("buildCompactToolSummary promotes file edits and todos to first-class classes", () => {
  assert.equal(
    buildCompactToolSummary(
      makeTool({ toolName: "str_replace", args: { path: "src/app.ts" } }),
    ).kind,
    "file-edit",
  );
  assert.equal(
    buildCompactToolSummary(makeTool({ toolName: "todo", args: { todos: [] } }))
      .kind,
    "plan",
  );
});
