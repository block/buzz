import assert from "node:assert/strict";
import test from "node:test";

import { linkDisplayText } from "./linkDisplay.ts";

test("linkDisplayText_stripsSchemeAndTrailingSlash", () => {
  assert.equal(linkDisplayText("https://example.com/"), "example.com");
  assert.equal(linkDisplayText("http://example.com/docs"), "example.com/docs");
});

test("linkDisplayText_keepsPathQueryAndFragment", () => {
  assert.equal(
    linkDisplayText("https://mail.google.com/mail/u/0/#drafts?compose=abc"),
    "mail.google.com/mail/u/0/#drafts?compose=abc",
  );
});

test("linkDisplayText_truncatesLongUrlsWithEllipsis", () => {
  const display = linkDisplayText(`https://example.com/${"a".repeat(100)}`);
  assert.equal(display.length, 64);
  assert.ok(display.endsWith("…"));
  assert.ok(display.startsWith("example.com/"));
});
