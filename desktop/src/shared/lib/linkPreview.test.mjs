import assert from "node:assert/strict";
import test from "node:test";

import {
  extractSupportedLinkPreviews,
  isSupportedLinkAutolinkLabel,
  parseSupportedLinkPreview,
} from "./linkPreview.ts";

test("parseSupportedLinkPreview parses GitHub pull request URLs", () => {
  assert.deepEqual(
    parseSupportedLinkPreview("https://github.com/block/sprout/pull/1234"),
    {
      kind: "github-pull-request",
      href: "https://github.com/block/sprout/pull/1234",
      provider: "GitHub",
      title: "block/sprout #1234",
      typeLabel: "PR",
    },
  );
});

test("parseSupportedLinkPreview parses GitHub repository URLs", () => {
  assert.deepEqual(
    parseSupportedLinkPreview("https://github.com/block/sprout"),
    {
      kind: "github-repository",
      href: "https://github.com/block/sprout",
      provider: "GitHub",
      title: "block/sprout",
      typeLabel: "repo",
    },
  );
});

test("parseSupportedLinkPreview trims markdown punctuation around GitHub URLs", () => {
  assert.deepEqual(
    parseSupportedLinkPreview("https://github.com/block/sprout/pull/1234)."),
    {
      kind: "github-pull-request",
      href: "https://github.com/block/sprout/pull/1234",
      provider: "GitHub",
      title: "block/sprout #1234",
      typeLabel: "PR",
    },
  );
});

test("parseSupportedLinkPreview ignores unsupported GitHub URLs", () => {
  assert.equal(
    parseSupportedLinkPreview("https://github.com/block/sprout/tree/main"),
    null,
  );
});

test("parseSupportedLinkPreview parses Linear issue URLs", () => {
  assert.deepEqual(
    parseSupportedLinkPreview(
      "https://linear.app/buzz/issue/BUG-321/fix-link-previews",
    ),
    {
      kind: "linear-issue",
      href: "https://linear.app/buzz/issue/BUG-321/fix-link-previews",
      provider: "Linear",
      title: "BUG-321",
      typeLabel: "issue",
    },
  );
});

test("parseSupportedLinkPreview normalizes Linear issue URL variants", () => {
  assert.deepEqual(
    parseSupportedLinkPreview("linear.app/buzz/issue/a-7/fix-link-previews"),
    {
      kind: "linear-issue",
      href: "https://linear.app/buzz/issue/a-7/fix-link-previews",
      provider: "Linear",
      title: "A-7",
      typeLabel: "issue",
    },
  );
});

test("parseSupportedLinkPreview parses Google app URLs", () => {
  assert.deepEqual(
    [
      "https://drive.google.com/file/d/abc123/view",
      "https://drive.google.com/drive/folders/folder123",
      "https://docs.google.com/document/d/doc123/edit",
      "https://docs.google.com/spreadsheets/d/sheet123/edit",
      "https://docs.google.com/presentation/d/slides123/edit",
    ].map((href) => parseSupportedLinkPreview(href)?.kind),
    [
      "google-drive-file",
      "google-drive-folder",
      "google-docs-document",
      "google-sheets-spreadsheet",
      "google-slides-presentation",
    ],
  );
});

test("extractSupportedLinkPreviews returns unique supported links in order", () => {
  assert.deepEqual(
    extractSupportedLinkPreviews(
      [
        "See github.com/block/sprout/pull/1",
        "and https://linear.app/buzz/issue/BUG-2/fix-preview",
        "then https://github.com/block/sprout/pull/1 again.",
        "plus https://docs.google.com/document/d/doc123/edit",
      ].join(" "),
    ).map((preview) => preview.title),
    ["block/sprout #1", "BUG-2", "Document"],
  );
});

test("extractSupportedLinkPreviews handles markdown link serialization", () => {
  assert.deepEqual(
    extractSupportedLinkPreviews(
      "[https://github.com/block/sprout/pull/44](https://github.com/block/sprout/pull/44)",
    ).map((preview) => preview.title),
    ["block/sprout #44"],
  );
});

test("extractSupportedLinkPreviews uses useful markdown labels as titles", () => {
  assert.deepEqual(
    extractSupportedLinkPreviews(
      "[Composer attachment polish](https://docs.google.com/document/d/doc123/edit)",
    ),
    [
      {
        kind: "google-docs-document",
        href: "https://docs.google.com/document/d/doc123/edit",
        provider: "Google Docs",
        title: "Composer attachment polish",
        typeLabel: "document",
      },
    ],
  );
});

test("extractSupportedLinkPreviews includes multiple supported Google links", () => {
  assert.deepEqual(
    extractSupportedLinkPreviews(
      [
        "https://docs.google.com/document/d/doc123/edit",
        "https://docs.google.com/spreadsheets/d/sheet123/edit",
        "https://docs.google.com/presentation/d/slides123/edit",
      ].join(" "),
    ).map((preview) => preview.kind),
    [
      "google-docs-document",
      "google-sheets-spreadsheet",
      "google-slides-presentation",
    ],
  );
});

test("extractSupportedLinkPreviews skips URLs inside inline and fenced code", () => {
  assert.deepEqual(
    extractSupportedLinkPreviews(
      [
        "`https://github.com/block/sprout/pull/1`",
        "```",
        "https://linear.app/buzz/issue/BUG-2/fix-preview",
        "```",
        "https://github.com/block/sprout/pull/3",
      ].join("\n"),
    ).map((preview) => preview.title),
    ["block/sprout #3"],
  );
});

test("isSupportedLinkAutolinkLabel matches normalized bare URL labels", () => {
  const preview = parseSupportedLinkPreview("github.com/block/sprout/pull/5");
  assert.ok(preview);
  assert.equal(
    isSupportedLinkAutolinkLabel(
      "https://github.com/block/sprout/pull/5",
      preview,
    ),
    true,
  );
  assert.equal(isSupportedLinkAutolinkLabel("review this", preview), false);
});
