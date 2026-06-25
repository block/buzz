import assert from "node:assert/strict";
import test from "node:test";

import { getLinkEditorInitialFocus } from "./linkEditorFocus.ts";

test("focuses URL when adding a link to selected text", () => {
  assert.equal(
    getLinkEditorInitialFocus({
      href: "",
      text: "selected text",
      from: 3,
      to: 16,
    }),
    "url",
  );
});

test("focuses display text when editing an existing link", () => {
  assert.equal(
    getLinkEditorInitialFocus({
      href: "https://example.com",
      text: "existing link",
      from: 3,
      to: 16,
    }),
    "text",
  );
});

test("focuses display text when inserting a link with no selected text", () => {
  assert.equal(
    getLinkEditorInitialFocus({
      href: "",
      text: "",
      from: 3,
      to: 3,
    }),
    "text",
  );
});

test("focuses display text when the selected text is whitespace only", () => {
  assert.equal(
    getLinkEditorInitialFocus({
      href: "",
      text: "   ",
      from: 3,
      to: 6,
    }),
    "text",
  );
});
