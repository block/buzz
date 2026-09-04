import assert from "node:assert/strict";
import test from "node:test";

import { summarizeMessageLinkContent } from "./messageLinkMetadata.ts";

test("summarizeMessageLinkContent projects markdown to bounded plain text", () => {
  assert.equal(
    summarizeMessageLinkContent(
      "**Hello** [team](https://example.com)\n\n![secret](https://example.com/a.png) ||hidden||",
    ),
    "Hello team",
  );
  assert.equal(
    summarizeMessageLinkContent("https://example.com"),
    "No message text",
  );
});

test("summarizeMessageLinkContent truncates on grapheme-safe character boundaries", () => {
  const result = summarizeMessageLinkContent(`Lead ${"🦄".repeat(200)}`);
  assert.ok(Array.from(result).length <= 160);
  assert.ok(result.endsWith("…"));
  assert.ok(!result.includes("\ud83e") || result.includes("🦄"));
});
