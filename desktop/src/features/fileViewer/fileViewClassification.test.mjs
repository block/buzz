import assert from "node:assert/strict";
import { test } from "node:test";

import { classifyFileView } from "./fileViewClassification.ts";

test("markdown extensions render as markdown regardless of MIME", () => {
  assert.deepEqual(classifyFileView("README.md", "application/octet-stream"), {
    kind: "markdown",
  });
  assert.deepEqual(classifyFileView("GUIDE.markdown"), { kind: "markdown" });
  assert.deepEqual(classifyFileView("Page.MDX"), { kind: "markdown" });
});
test("code extensions map to Shiki language ids", () => {
  assert.deepEqual(classifyFileView("apply-config.sh"), {
    kind: "code",
    language: "shellscript",
  });
  assert.deepEqual(classifyFileView("main.rs"), {
    kind: "code",
    language: "rust",
  });
  assert.deepEqual(classifyFileView("data.yml"), {
    kind: "code",
    language: "yaml",
  });
  assert.deepEqual(classifyFileView("Config.TOML"), {
    kind: "code",
    language: "toml",
  });
});

// The Shiki registry is what makes the long tail work without a hand table;
// these are languages my original map never listed.
test("languages beyond the hand-written set resolve from the Shiki registry", () => {
  for (const [filename, language] of [
    ["main.zig", "zig"],
    ["cluster.tf", "terraform"],
    ["query.sparql", "sparql"],
    // `dockerfile` is an alias; the grammar id is `docker`.
    ["Dockerfile.dockerfile", "docker"],
    ["notebook.jl", "julia"],
    ["module.erl", "erlang"],
    ["view.hbs", "handlebars"],
    ["schema.prisma", "prisma"],
  ]) {
    assert.deepEqual(
      classifyFileView(filename),
      { kind: "code", language },
      filename,
    );
  }
});

// Extensions Shiki does not alias; regressions here silently lose highlighting.
test("override table covers extensions Shiki does not alias", () => {
  assert.deepEqual(classifyFileView("stack.h"), {
    kind: "code",
    language: "c",
  });
  assert.deepEqual(classifyFileView("stack.hpp"), {
    kind: "code",
    language: "cpp",
  });
  assert.deepEqual(classifyFileView("fix.patch"), {
    kind: "code",
    language: "diff",
  });
  assert.deepEqual(classifyFileView("server.mjs"), {
    kind: "code",
    language: "javascript",
  });
});

// Real filenames an agent delivered in a channel — these must be viewable
// even though the relay stored them as `application/octet-stream`.
test("agent-delivered attachments classify from the filename, not the MIME", () => {
  assert.deepEqual(
    classifyFileView("power_law_btc_analysis.py", "application/octet-stream"),
    { kind: "code", language: "python" },
  );
  assert.deepEqual(
    classifyFileView("market_breadth_history.json", "application/octet-stream"),
    { kind: "code", language: "json" },
  );
  assert.deepEqual(
    classifyFileView(
      "architecture-hermes-agent-organization.md",
      "application/octet-stream",
    ),
    { kind: "markdown" },
  );
});

test("plain-text extensions render as text", () => {
  assert.deepEqual(classifyFileView("build.log"), { kind: "text" });
  assert.deepEqual(classifyFileView("data.csv"), { kind: "text" });
});

test("binary/container types are not viewable", () => {
  assert.deepEqual(classifyFileView("Q3-budget.pdf", "application/pdf"), {
    kind: "none",
  });
  assert.deepEqual(classifyFileView("archive.zip", "application/zip"), {
    kind: "none",
  });
  assert.deepEqual(classifyFileView("photo.png", "image/png"), {
    kind: "none",
  });
});

test("unknown extension falls back to the imeta MIME", () => {
  assert.deepEqual(classifyFileView("notes", "text/markdown"), {
    kind: "markdown",
  });
  assert.deepEqual(classifyFileView("payload", "application/json"), {
    kind: "code",
    language: "json",
  });
  assert.deepEqual(classifyFileView("report", "text/plain; charset=utf-8"), {
    kind: "text",
  });
  assert.deepEqual(classifyFileView("blob", "application/octet-stream"), {
    kind: "none",
  });
  assert.deepEqual(classifyFileView("blob"), { kind: "none" });
});

test("a trailing dot yields no extension and defers to MIME", () => {
  assert.deepEqual(classifyFileView("weird.", "text/plain"), { kind: "text" });
});
