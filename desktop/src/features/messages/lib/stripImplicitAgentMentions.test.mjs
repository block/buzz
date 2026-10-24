import assert from "node:assert/strict";
import test from "node:test";

import { stripImplicitAgentMentionPrefix } from "./stripImplicitAgentMentions.ts";

test("removes the exact synthesized leading prefix", () => {
  assert.equal(
    stripImplicitAgentMentionPrefix("@Morgarita draft text", "@Morgarita "),
    "draft text",
  );
});

test("removes the complete captured prefix for multiple agents", () => {
  assert.equal(
    stripImplicitAgentMentionPrefix(
      "@Morgarita @Vogue draft text",
      "@Morgarita @Vogue ",
    ),
    "draft text",
  );
});

test("removes an implicit-only mention when markdown drops its separator", () => {
  assert.equal(
    stripImplicitAgentMentionPrefix("@Morgarita", "@Morgarita "),
    "",
  );
});

test("preserves an identical authored mention after the synthesized prefix", () => {
  assert.equal(
    stripImplicitAgentMentionPrefix(
      "@Morgarita @Morgarita authored duplicate",
      "@Morgarita ",
    ),
    "@Morgarita authored duplicate",
  );
});

test("preserves content when the captured prefix does not match exactly", () => {
  assert.equal(
    stripImplicitAgentMentionPrefix("@Alice ask @Morgarita", "@Morgarita "),
    "@Alice ask @Morgarita",
  );
});
