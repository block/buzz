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
  assert.deepEqual(summary.action, { verb: "Ran", object: "git status" });
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

  assert.equal(summary.kind, "image");
  assert.equal(summary.label, "Viewed image");
  assert.equal(summary.thumbnailSrc, source);
  assert.equal(summary.imageContent?.src, source);
  assert.equal(summary.preview, source);
});

test("buildCompactToolSummary uses basename for local view_image paths", () => {
  const summary = buildCompactToolSummary(
    makeTool({
      toolName: "view_image",
      args: { source: "desktop/assets/screenshot.png" },
    }),
  );

  assert.equal(summary.kind, "image");
  assert.equal(summary.thumbnailSrc, null);
  assert.equal(summary.imageContent, null);
  assert.equal(summary.preview, "screenshot.png");
});

test("buildCompactToolSummary formats read_file path preview", () => {
  const path = "desktop/src/app/App.tsx";
  const summary = buildCompactToolSummary(
    makeTool({
      toolName: "read_file",
      args: { path },
      result: `${path} (lines 1-2 of 2)\n1:export {}\n2: `,
    }),
  );

  assert.equal(summary.kind, "file-read");
  assert.equal(summary.label, "Read file");
  assert.equal(summary.preview, path);
  assert.deepEqual(summary.action, {
    verb: "Read",
    object: path,
  });
  assert.ok(summary.fileReadContent);
});

test("buildCompactToolSummary formats load_skill into skill-read file panel", () => {
  const summary = buildCompactToolSummary(
    makeTool({
      toolName: "load_skill",
      args: { name: "block-safe-github" },
      result: "# Safe GitHub usage at Block\n",
    }),
  );

  assert.equal(summary.kind, "skill-read");
  assert.equal(summary.label, "Read skill");
  assert.equal(summary.preview, "block-safe-github");
  assert.deepEqual(summary.action, {
    verb: "Read",
    object: "block-safe-github",
  });
  assert.ok(summary.fileReadContent);
  assert.equal(
    summary.fileReadContent?.footerText,
    "block-safe-github/SKILL.md",
  );
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
  assert.deepEqual(summary.action, { verb: "Read", object: "channel-1" });
  assert.equal(summary.presentation, "inline");
  assert.equal(summary.shellContent, "buzz channels get --channel channel-1");
});

test("buildCompactToolSummary exposes shellContent for shell-sourced buzz CLI reads", () => {
  const command =
    "sleep 45; buzz messages thread --channel channel-uuid --event abc | tail -n 20";
  const summary = buildCompactToolSummary(
    makeTool({
      toolName: "shell",
      args: { command },
      result: JSON.stringify({
        stdout: "[1782969453] user: hello",
        exit_code: 0,
      }),
    }),
  );

  assert.equal(summary.kind, "relay-op");
  assert.equal(summary.shellContent, command);
  assert.deepEqual(summary.action, {
    verb: "Read",
    object: "channel-uuid",
  });
});

test("buildCompactToolSummary derives structured actions for native Buzz MCP tools", () => {
  const summary = buildCompactToolSummary(
    makeTool({
      toolName: "get_channel",
      buzzToolName: "get_channel",
      args: {
        channel_id: "channel-1",
      },
    }),
  );

  assert.equal(summary.kind, "relay-op");
  assert.deepEqual(summary.action, { verb: "Read", object: "channel-1" });
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

test("buildCompactToolSummary formats file edits as filename plus diff stats", () => {
  const summary = buildCompactToolSummary(
    makeTool({
      toolName: "str_replace",
      args: { path: "desktop/src/app/App.tsx" },
      result: [
        "Replaced 1 occurrence.",
        "",
        "--- a/desktop/src/app/App.tsx",
        "+++ b/desktop/src/app/App.tsx",
        "@@",
        "-<Switch />",
        "+<DropdownMenuCheckboxItem />",
        "+<DropdownMenuSeparator />",
      ].join("\n"),
    }),
  );

  assert.equal(summary.kind, "file-edit");
  assert.equal(summary.preview, "App.tsx");
  assert.deepEqual(summary.fileEditSummary, {
    path: "desktop/src/app/App.tsx",
    filename: "App.tsx",
    additions: 2,
    deletions: 1,
  });
  assert.deepEqual(summary.fileEditDiff?.lines, [
    { kind: "meta", text: "--- a/desktop/src/app/App.tsx" },
    { kind: "meta", text: "+++ b/desktop/src/app/App.tsx" },
    { kind: "meta", text: "@@" },
    { kind: "remove", text: "-<Switch />" },
    { kind: "add", text: "+<DropdownMenuCheckboxItem />" },
    { kind: "add", text: "+<DropdownMenuSeparator />" },
  ]);
});

test("buildCompactToolSummary counts Shiki diff markers for file edit stats", () => {
  const summary = buildCompactToolSummary(
    makeTool({
      toolName: "str_replace",
      args: { path: "desktop/src/app/App.tsx" },
      result: [
        "const keep = true;",
        "const next = true; // [!code ++]",
        "const old = true; // [!code --]",
      ].join("\n"),
    }),
  );

  assert.deepEqual(summary.fileEditSummary, {
    path: "desktop/src/app/App.tsx",
    filename: "App.tsx",
    additions: 1,
    deletions: 1,
  });
  assert.deepEqual(summary.fileEditDiff?.lines, [
    { kind: "context", text: "const keep = true;" },
    { kind: "add", text: "const next = true;" },
    { kind: "remove", text: "const old = true;" },
  ]);
});

test("buildCompactToolSummary parses file edit stats from shell JSON stdout", () => {
  const summary = buildCompactToolSummary(
    makeTool({
      toolName: "str_replace",
      args: { path: "desktop/src/app/App.tsx" },
      result: JSON.stringify({
        stdout: [
          "diff --git a/desktop/src/app/App.tsx b/desktop/src/app/App.tsx",
          "--- a/desktop/src/app/App.tsx",
          "+++ b/desktop/src/app/App.tsx",
          "@@",
          "-old",
          "+new",
        ].join("\n"),
      }),
    }),
  );

  assert.deepEqual(summary.fileEditSummary, {
    path: "desktop/src/app/App.tsx",
    filename: "App.tsx",
    additions: 1,
    deletions: 1,
  });
  assert.deepEqual(summary.fileEditDiff?.lines, [
    {
      kind: "meta",
      text: "diff --git a/desktop/src/app/App.tsx b/desktop/src/app/App.tsx",
    },
    { kind: "meta", text: "--- a/desktop/src/app/App.tsx" },
    { kind: "meta", text: "+++ b/desktop/src/app/App.tsx" },
    { kind: "meta", text: "@@" },
    { kind: "remove", text: "-old" },
    { kind: "add", text: "+new" },
  ]);
});

test("buildCompactToolSummary trims only trailing blank diff lines", () => {
  const summary = buildCompactToolSummary(
    makeTool({
      toolName: "str_replace",
      args: { path: "desktop/src/app/App.tsx" },
      result: [
        "--- a/desktop/src/app/App.tsx",
        "+++ b/desktop/src/app/App.tsx",
        "@@",
        " const before = true;",
        "",
        "+const after = true;",
        "",
      ].join("\n"),
    }),
  );

  assert.deepEqual(summary.fileEditDiff?.lines, [
    { kind: "meta", text: "--- a/desktop/src/app/App.tsx" },
    { kind: "meta", text: "+++ b/desktop/src/app/App.tsx" },
    { kind: "meta", text: "@@" },
    { kind: "context", text: " const before = true;" },
    { kind: "context", text: "" },
    { kind: "add", text: "+const after = true;" },
  ]);
});
