import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { buildCompactToolSummary } from "../agentSessionToolSummary.ts";
import { CompactToolSummaryRow } from "./CompactToolSummaryRow.tsx";

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

/** Build the row via the real summary pipeline, then render it to HTML. */
function renderRow(item, { summaryTitleEnabled = true } = {}) {
  const summary = buildCompactToolSummary(item, { summaryTitleEnabled });
  return renderToStaticMarkup(
    React.createElement(CompactToolSummaryRow, {
      action: summary.action,
      duration: null,
      failed: summary.failed,
      fileEditSummary: summary.fileEditSummary,
      kind: summary.kind,
      label: summary.label,
      preview: summary.preview,
      summaryTitle: summary.summaryTitle,
      thumbnailSrc: summary.thumbnailSrc,
    }),
  );
}

test("render: friendly summary title is the visible label for an action-bearing shell row", () => {
  const html = renderRow(
    makeTool({
      toolName: "developer__shell",
      args: { command: "git status" },
      summaryTitle: "checking repository state",
    }),
  );

  assert.ok(
    html.includes("checking repository state"),
    `friendly title must be painted, got: ${html}`,
  );
  // The structured "Ran <command>" descriptor must not win the row label.
  assert.ok(
    !html.includes(">Ran<"),
    `descriptor verb must not render: ${html}`,
  );
  // Receipt stays available: the exact command rides the hover title.
  assert.ok(
    html.includes('title="git status"'),
    `exact command receipt must stay on hover: ${html}`,
  );
});

test("render: friendly summary title is the visible label for an action-bearing file-read row", () => {
  const html = renderRow(
    makeTool({
      toolName: "read_file",
      args: { path: "crates/buzz-agent/src/agent.rs" },
      summaryTitle: "reading agent source module",
    }),
  );

  assert.ok(html.includes("reading agent source module"));
  assert.ok(
    !html.includes(">Read<"),
    `descriptor verb must not render: ${html}`,
  );
});

test("render: failed shell row ignores the friendly title and paints the failure label", () => {
  const html = renderRow(
    makeTool({
      toolName: "developer__shell",
      args: { command: "false" },
      status: "failed",
      isError: true,
      result: "exit 1",
      summaryTitle: "running a quick command",
    }),
  );

  assert.ok(
    !html.includes("running a quick command"),
    `friendly phrase must not mask a failure: ${html}`,
  );
  assert.ok(
    html.includes("failed"),
    `failure label must be visible, got: ${html}`,
  );
});

test("render: rows without a summary keep today's descriptor label", () => {
  const html = renderRow(
    makeTool({
      toolName: "developer__shell",
      args: { command: "git status" },
    }),
  );

  assert.ok(html.includes(">Ran<"), `descriptor verb should render: ${html}`);
  assert.ok(html.includes("git status"));
});

test("render: experiment off ignores the friendly title and paints the raw descriptor", () => {
  const html = renderRow(
    makeTool({
      toolName: "developer__shell",
      args: { command: "git status" },
      summaryTitle: "checking repository state",
    }),
    { summaryTitleEnabled: false },
  );

  assert.ok(
    !html.includes("checking repository state"),
    `friendly title must not paint when the experiment is off: ${html}`,
  );
  assert.ok(
    html.includes(">Ran<"),
    `raw descriptor verb must render when the experiment is off: ${html}`,
  );
  assert.ok(html.includes("git status"));
});

test("render: experiment off keeps the raw file-read label", () => {
  const html = renderRow(
    makeTool({
      toolName: "read_file",
      args: { path: "crates/buzz-agent/src/agent.rs" },
      summaryTitle: "reading agent source module",
    }),
    { summaryTitleEnabled: false },
  );

  assert.ok(
    !html.includes("reading agent source module"),
    `friendly title must not paint when the experiment is off: ${html}`,
  );
  assert.ok(
    html.includes(">Read<"),
    `raw descriptor verb must render when the experiment is off: ${html}`,
  );
});
