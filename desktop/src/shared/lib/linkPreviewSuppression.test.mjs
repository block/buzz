import assert from "node:assert/strict";
import test from "node:test";
import {
  buildLinkPreviewSuppressionTags,
  getLinkPreviewSuppression,
  normalizeSuppressedLinkPreviewUrls,
} from "./linkPreviewSuppression.mjs";

test("normalizes, de-duplicates, and rejects unsafe preview suppression URLs", () => {
  assert.deepEqual(
    normalizeSuppressedLinkPreviewUrls([
      "https://EXAMPLE.com/path#one",
      "https://example.com/path#two",
      "http://example.com",
      "nope",
    ]),
    ["https://example.com/path"],
  );
});

test("parses legacy blanket and URL-specific suppression independently", () => {
  assert.deepEqual(
    getLinkPreviewSuppression([
      ["link-preview", "none"],
      ["link-preview", "hide", "https://example.com/a#x"],
      ["link-preview", "hide", "javascript:alert(1)"],
    ]),
    { all: true, urls: ["https://example.com/a"] },
  );
});

test("builds one deterministic tag per canonical URL", () => {
  assert.deepEqual(
    buildLinkPreviewSuppressionTags([
      "https://example.com/a#x",
      "https://example.com/a#y",
    ]),
    [["link-preview", "hide", "https://example.com/a"]],
  );
});
