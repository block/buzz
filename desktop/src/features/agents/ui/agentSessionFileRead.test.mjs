import assert from "node:assert/strict";
import test from "node:test";

import { buildFileReadContent } from "./agentSessionFileRead.ts";

const baseDescriptor = {
  renderClass: "file-read",
  label: "Read file",
  preview: "src/App.tsx",
  groupKey: "read_file",
};

function makeTool(overrides = {}) {
  return {
    id: "tool:1",
    type: "tool",
    title: "read_file",
    toolName: "read_file",
    buzzToolName: null,
    status: "completed",
    args: { path: "src/App.tsx" },
    result: "",
    isError: false,
    timestamp: "2026-06-14T19:00:00.000Z",
    startedAt: "2026-06-14T19:00:00.000Z",
    completedAt: "2026-06-14T19:00:01.000Z",
    descriptor: baseDescriptor,
    ...overrides,
  };
}

test("buildFileReadContent returns null for non file-read render class", () => {
  assert.equal(
    buildFileReadContent(makeTool(), {
      ...baseDescriptor,
      renderClass: "generic",
    }),
    null,
  );
});

test("buildFileReadContent parses range header and meta footer", () => {
  const path = "src/App.tsx";
  const result = [
    `${path} (lines 81-300 of 438)`,
    "81:export function App() {",
    "82:  return null;",
    "[showing lines 81-300 of 438; use offset=300 to continue]",
  ].join("\n");

  const content = buildFileReadContent(
    makeTool({ args: { path }, result }),
    baseDescriptor,
  );

  assert.ok(content);
  assert.equal(content.path, path);
  assert.equal(content.footerText, `${path} (lines 81-300 of 438)`);
  assert.equal(content.lines.length, 3);
  assert.equal(content.lines[0]?.kind, "context");
  assert.equal(content.lines[2]?.kind, "meta");
});

test("buildFileReadContent handles empty result text", () => {
  assert.equal(
    buildFileReadContent(makeTool({ result: "   " }), baseDescriptor),
    null,
  );
});
