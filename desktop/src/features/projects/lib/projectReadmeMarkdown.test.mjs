import assert from "node:assert/strict";
import { test } from "node:test";

import { normalizeReadmeMarkdown } from "./projectReadmeMarkdown.ts";

test("normalizes the supported README HTML subset", () => {
  assert.equal(
    normalizeReadmeMarkdown(
      '<h2>Hello &amp; welcome</h2><p><strong>Bold</strong><br><a href="https://example.com/a path">Docs</a></p>',
    ),
    "## Hello & welcome\n\n**Bold**\n[Docs](https://example.com/a%20path)",
  );
});

test("renders unknown and entity-encoded HTML as inert text", () => {
  const normalized = normalizeReadmeMarkdown(
    "<p>&lt;script&gt;one()&lt;/script&gt;<scr<script>ipt>two()</scr<script>ipt></p>",
  );

  assert.doesNotMatch(normalized, /<script/i);
  assert.match(normalized, /&lt;script&gt;one\(\)&lt;\/script&gt;/);
  assert.match(normalized, /&lt;scr&lt;script&gt;ipt&gt;two\(\)/);
});

test("decodes only one entity layer", () => {
  assert.equal(
    normalizeReadmeMarkdown("&amp;lt;script&amp;gt;alert(1)"),
    "&lt;script&gt;alert(1)",
  );
});

test("drops unsafe link and image destinations without dropping labels", () => {
  const normalized = normalizeReadmeMarkdown(
    '<p><a href="java&#x73;cript:alert(1)">Open me</a><img alt="payload" src="data:text/html,boom"></p>',
  );

  assert.equal(normalized, "Open me");
  assert.doesNotMatch(normalized, /javascript:|data:text\/html/i);
});
