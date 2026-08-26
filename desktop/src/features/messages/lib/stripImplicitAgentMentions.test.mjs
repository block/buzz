import assert from "node:assert/strict";
import test from "node:test";

import { stripImplicitAgentMentions } from "./stripImplicitAgentMentions.ts";

test("removes a synthesized leading agent mention", () => {
  assert.equal(
    stripImplicitAgentMentions("@Morgarita draft text", ["Morgarita"]),
    "draft text",
  );
});

test("removes the complete synthesized prefix for multiple agents", () => {
  assert.equal(
    stripImplicitAgentMentions("@Morgarita @Vogue draft text", [
      "Morgarita",
      "Vogue",
    ]),
    "draft text",
  );
});

test("removes an implicit-only mention after markdown drops its separator", () => {
  assert.equal(stripImplicitAgentMentions("@Morgarita", ["Morgarita"]), "");
});

test("preserves matching mentions outside the synthesized prefix", () => {
  assert.equal(
    stripImplicitAgentMentions("draft for @Morgarita", ["Morgarita"]),
    "draft for @Morgarita",
  );
});

test("preserves an unknown leading mention", () => {
  assert.equal(
    stripImplicitAgentMentions("@Alice ask @Morgarita", ["Morgarita"]),
    "@Alice ask @Morgarita",
  );
});
