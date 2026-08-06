import assert from "node:assert/strict";
import test from "node:test";

import { reactionGlyphPresentation } from "./reactionGlyphPresentation.ts";

test("uses compact layout only for one native emoji cluster", () => {
  for (const emoji of ["😀", "❤️", "👍🏽", "👨‍👩‍👧‍👦", "🇺🇸"]) {
    assert.deepEqual(reactionGlyphPresentation(emoji), {
      kind: "native",
      text: emoji,
    });
  }

  for (const text of ["a", "ship it", "😀😀"]) {
    assert.deepEqual(reactionGlyphPresentation(text), { kind: "text", text });
  }
});

test("only unwraps valid outer shortcode delimiters for text fallbacks", () => {
  assert.deepEqual(reactionGlyphPresentation(":bozo:"), {
    kind: "text",
    text: "bozo",
  });
  assert.deepEqual(reactionGlyphPresentation(":party_parrot:"), {
    kind: "text",
    text: "party_parrot",
  });
  for (const text of [":ship it:", "::", ":bozo", "bozo:"]) {
    assert.deepEqual(reactionGlyphPresentation(text), { kind: "text", text });
  }
});
