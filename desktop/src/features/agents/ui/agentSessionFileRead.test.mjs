import assert from "node:assert/strict";
import test from "node:test";

import {
  buildFileReadContent,
  buildSkillReadContent,
} from "./agentSessionFileRead.ts";

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

const skillDescriptor = {
  renderClass: "skill-read",
  label: "Read skill",
  preview: "block-safe-github",
  groupKey: "skill:load",
};

test("buildSkillReadContent returns null for non skill-read render class", () => {
  assert.equal(
    buildSkillReadContent(
      makeTool({
        toolName: "load_skill",
        args: { name: "block-safe-github" },
      }),
      baseDescriptor,
    ),
    null,
  );
});

test("buildSkillReadContent maps skill body into file content panel", () => {
  const content = buildSkillReadContent(
    makeTool({
      toolName: "load_skill",
      args: { name: "block-safe-github" },
      result:
        "# Safe GitHub usage at Block\n\nAll Block code must live in org repos.",
      descriptor: skillDescriptor,
    }),
    skillDescriptor,
  );

  assert.ok(content);
  assert.equal(content.path, "block-safe-github");
  assert.equal(content.footerText, "block-safe-github/SKILL.md");
  assert.equal(content.lines.length, 3);
  assert.equal(content.lines[0]?.text, "# Safe GitHub usage at Block");
  assert.equal(
    content.lines[2]?.text,
    "All Block code must live in org repos.",
  );
});

test("buildSkillReadContent uses the supporting-file path in the footer", () => {
  const skillRef = "block-safe-github/references/foo.md";
  const content = buildSkillReadContent(
    makeTool({
      toolName: "load_skill",
      args: { name: skillRef },
      result: "# Reference\n",
      descriptor: {
        ...skillDescriptor,
        label: "Read skill file",
        preview: skillRef,
        groupKey: "skill:load-file",
      },
    }),
    {
      ...skillDescriptor,
      label: "Read skill file",
      preview: skillRef,
      groupKey: "skill:load-file",
    },
  );

  assert.ok(content);
  assert.equal(content.footerText, skillRef);
});
