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

test("parseSupportedLinkPreview strips the fragment from the preview href", () => {
  // A `#fragment` is a client-only anchor; the preview and its signed snapshot
  // canonical URL are of the page. Keeping it would fail the fragmentless
  // snapshot-URL guard and drop the preview entirely.
  assert.equal(
    parseSupportedLinkPreview(
      "https://github.com/block/sprout/pull/1234#pullrequestreview-99",
    )?.href,
    "https://github.com/block/sprout/pull/1234",
  );
});

test("extractSupportedLinkPreviews collapses fragment variants of one page", () => {
  const previews = extractSupportedLinkPreviews(
    [
      "https://github.com/block/sprout/pull/1234#pullrequestreview-99",
      "https://github.com/block/sprout/pull/1234#issuecomment-1",
      "https://github.com/block/sprout/pull/5678",
    ].join("\n"),
  );
  // Two anchors into the same page dedupe to one card at first occurrence; the
  // distinct second page keeps its own card.
  assert.deepEqual(
    previews.map((preview) => preview.href),
    [
      "https://github.com/block/sprout/pull/1234",
      "https://github.com/block/sprout/pull/5678",
    ],
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

const BUZZ_OWNER =
  "71d67180ba17e749ee825fc8819c9c6ee7003617e1c126504f9b658070ab9224";

test("parseSupportedLinkPreview parses Buzz relay git clone URLs", () => {
  // Must pass the active relay origin for host validation.
  assert.deepEqual(
    parseSupportedLinkPreview(
      `https://buzz.block.builderlab.xyz/git/${BUZZ_OWNER}/buzz-world-galaxy`,
      "https://buzz.block.builderlab.xyz",
    ),
    {
      kind: "buzz-repository",
      href: `buzz://repo?owner=${BUZZ_OWNER}&d=buzz-world-galaxy`,
      provider: "Buzz",
      title: "buzz-world-galaxy",
      typeLabel: "repo",
    },
  );
  // Same URL without a matching origin stays an ordinary external preview.
  assert.equal(
    parseSupportedLinkPreview(
      `https://buzz.block.builderlab.xyz/git/${BUZZ_OWNER}/buzz-world-galaxy`,
    )?.kind,
    "generic-link",
  );
});

test("parseSupportedLinkPreview strips .git suffix from clone URLs", () => {
  assert.deepEqual(
    parseSupportedLinkPreview(
      `http://localhost:3000/git/${BUZZ_OWNER}/buzz-world.git`,
      "http://localhost:3000",
    ),
    {
      kind: "buzz-repository",
      href: `buzz://repo?owner=${BUZZ_OWNER}&d=buzz-world`,
      provider: "Buzz",
      title: "buzz-world",
      typeLabel: "repo",
    },
  );
});

test("parseSupportedLinkPreview rejects malformed Buzz git URLs", () => {
  for (const href of [
    // Owner segment must be a 64-char lowercase hex pubkey.
    "https://relay.example/git/not-a-pubkey/repo",
    `https://relay.example/git/${BUZZ_OWNER.toUpperCase()}/repo`,
    `https://relay.example/git/${BUZZ_OWNER.slice(0, 32)}/repo`,
    // Missing or invalid repo segment.
    `https://relay.example/git/${BUZZ_OWNER}`,
    `https://relay.example/git/${BUZZ_OWNER}/.hidden`,
    // Deeper transport paths are not repo links.
    `https://relay.example/git/${BUZZ_OWNER}/repo/info/refs`,
  ]) {
    // Structural non-matches remain ordinary external previews.
    assert.equal(
      parseSupportedLinkPreview(href, "https://relay.example")?.kind,
      "generic-link",
      href,
    );
  }
});

test("parseSupportedLinkPreview rejects clone URLs from non-relay hosts", () => {
  // Correct path shape but origin does not match the active relay.
  assert.equal(
    parseSupportedLinkPreview(
      `https://evil.example/git/${BUZZ_OWNER}/my-repo`,
      "https://buzz.block.builderlab.xyz",
    )?.kind,
    "generic-link",
  );
  // github.com sharing the path shape must never become a Buzz repo card.
  assert.equal(
    parseSupportedLinkPreview(
      `https://github.com/git/${BUZZ_OWNER}/my-repo`,
      "https://buzz.block.builderlab.xyz",
    ),
    null,
  );
  // No relay origin provided — stays external.
  assert.equal(
    parseSupportedLinkPreview(
      `https://buzz.block.builderlab.xyz/git/${BUZZ_OWNER}/buzz-world`,
      null,
    )?.kind,
    "generic-link",
  );
});

const BUZZ_EVENT_ID =
  "c3b589fa5713ba25bad6dc095e2de00a4ac8f50050fdea00fc6444e603be1dd1";

test("parseSupportedLinkPreview parses buzz:// PR and issue deep links", () => {
  assert.deepEqual(
    parseSupportedLinkPreview(
      `buzz://pr?id=${BUZZ_EVENT_ID}&owner=${BUZZ_OWNER}&d=buzz-world`,
    ),
    {
      kind: "buzz-pull-request",
      href: `buzz://pr?id=${BUZZ_EVENT_ID}&owner=${BUZZ_OWNER}&d=buzz-world`,
      provider: "Buzz",
      title: "buzz-world #c3b589fa",
      typeLabel: "PR",
    },
  );
  assert.deepEqual(
    parseSupportedLinkPreview(
      `buzz://issue?id=${BUZZ_EVENT_ID}&owner=${BUZZ_OWNER}&d=buzz-world`,
    )?.typeLabel,
    "issue",
  );
  assert.deepEqual(
    parseSupportedLinkPreview(`buzz://repo?owner=${BUZZ_OWNER}&d=buzz-world`),
    {
      kind: "buzz-repository",
      href: `buzz://repo?owner=${BUZZ_OWNER}&d=buzz-world`,
      provider: "Buzz",
      title: "buzz-world",
      typeLabel: "repo",
    },
  );
});

test("parseSupportedLinkPreview rejects malformed buzz:// entity links", () => {
  for (const href of [
    `buzz://pr?owner=${BUZZ_OWNER}&d=buzz-world`,
    `buzz://pr?id=short&owner=${BUZZ_OWNER}&d=buzz-world`,
    `buzz://issue?id=${BUZZ_EVENT_ID}&owner=nope&d=buzz-world`,
    `buzz://repo?owner=${BUZZ_OWNER}&d=.hidden`,
  ]) {
    assert.equal(parseSupportedLinkPreview(href), null, href);
  }
});

test("extractSupportedLinkPreviews picks up buzz:// links in prose", () => {
  assert.deepEqual(
    extractSupportedLinkPreviews(
      `PR is up: buzz://pr?id=${BUZZ_EVENT_ID}&owner=${BUZZ_OWNER}&d=buzz-world — review please.`,
    ).map((preview) => [preview.kind, preview.title]),
    [["buzz-pull-request", "buzz-world #c3b589fa"]],
  );
});

test("extractSupportedLinkPreviews uses markdown labels for buzz:// links", () => {
  assert.deepEqual(
    extractSupportedLinkPreviews(
      `[Add header links](buzz://pr?id=${BUZZ_EVENT_ID}&owner=${BUZZ_OWNER}&d=buzz-world)`,
    ).map((preview) => preview.title),
    ["Add header links"],
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

test("extractSupportedLinkPreviews picks up bare Buzz clone URLs in prose", () => {
  assert.deepEqual(
    extractSupportedLinkPreviews(
      `master pushed; clone: https://buzz.block.builderlab.xyz/git/${BUZZ_OWNER}/buzz-world-galaxy and review please.`,
      "https://buzz.block.builderlab.xyz",
    ),
    [
      {
        kind: "buzz-repository",
        href: `buzz://repo?owner=${BUZZ_OWNER}&d=buzz-world-galaxy`,
        provider: "Buzz",
        title: "buzz-world-galaxy",
        typeLabel: "repo",
      },
    ],
  );
  // Without a relay origin the URL is treated as an ordinary external link.
  assert.deepEqual(
    extractSupportedLinkPreviews(
      `clone: https://buzz.block.builderlab.xyz/git/${BUZZ_OWNER}/buzz-world-galaxy`,
    ).map((preview) => preview.kind),
    ["generic-link"],
  );
});

test("extractSupportedLinkPreviews uses markdown labels for Buzz repo links", () => {
  assert.deepEqual(
    extractSupportedLinkPreviews(
      `[Buzz World](https://relay.example/git/${BUZZ_OWNER}/buzz-world-galaxy)`,
      "https://relay.example",
    ).map((preview) => preview.title),
    ["Buzz World"],
  );
});

test("extractSupportedLinkPreviews dedupes clone URL variants of one repo", () => {
  assert.deepEqual(
    extractSupportedLinkPreviews(
      [
        `https://relay.example/git/${BUZZ_OWNER}/buzz-world-galaxy`,
        `https://relay.example/git/${BUZZ_OWNER}/buzz-world-galaxy.git`,
      ].join(" "),
      "https://relay.example",
    ).map((preview) => preview.href),
    [`buzz://repo?owner=${BUZZ_OWNER}&d=buzz-world-galaxy`],
  );
});

test("clone URLs and buzz://repo links for the same repo dedupe to one card", () => {
  assert.deepEqual(
    extractSupportedLinkPreviews(
      [
        `https://relay.example/git/${BUZZ_OWNER}/buzz-world-galaxy`,
        `buzz://repo?owner=${BUZZ_OWNER}&d=buzz-world-galaxy`,
      ].join(" "),
      "https://relay.example",
    ).map((preview) => preview.href),
    [`buzz://repo?owner=${BUZZ_OWNER}&d=buzz-world-galaxy`],
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

test("extractSupportedLinkPreviews skips URLs inside indented code", () => {
  assert.deepEqual(
    extractSupportedLinkPreviews(
      [
        "    https://docs.google.com/document/d/hidden/edit",
        "\tgithub.com/block/sprout/pull/4",
        "https://github.com/block/sprout/pull/5",
      ].join("\n"),
    ).map((preview) => preview.title),
    ["block/sprout #5"],
  );
});

test("extractSupportedLinkPreviews skips markdown image link URLs", () => {
  assert.deepEqual(
    extractSupportedLinkPreviews(
      [
        "![alt](https://docs.google.com/document/d/doc123/edit)",
        "![alt](https://github.com/block/sprout)",
        "[Composer attachment polish](https://docs.google.com/document/d/doc456/edit)",
      ].join("\n"),
    ).map((preview) => preview.title),
    ["Composer attachment polish"],
  );
});

test("extractSupportedLinkPreviews treats other absolute HTTPS URLs as generic", () => {
  assert.deepEqual(
    extractSupportedLinkPreviews(
      [
        "https://evil-github.com/block/sprout/pull/1",
        "https://example.com/go/https://docs.google.com/document/d/doc123/edit",
        "(https://github.com/block/sprout/pull/2)",
      ].join(" "),
    ).map((preview) => preview.title),
    ["evil-github.com", "example.com", "block/sprout #2"],
  );
});

test("extractSupportedLinkPreviews skips links inside inline spoilers", () => {
  assert.deepEqual(
    extractSupportedLinkPreviews(
      [
        "Keep",
        "||[roadmap](https://docs.google.com/document/d/hidden/edit)||",
        "hidden, but show https://github.com/block/sprout/pull/7",
      ].join(" "),
    ).map((preview) => preview.title),
    ["block/sprout #7"],
  );
});

test("extractSupportedLinkPreviews skips links inside block spoilers", () => {
  assert.deepEqual(
    extractSupportedLinkPreviews(
      [
        "||",
        "",
        "https://linear.app/buzz/issue/BUG-99/hidden-spoiler-link",
        "",
        "||",
        "https://github.com/block/sprout/pull/8",
      ].join("\n"),
    ).map((preview) => preview.title),
    ["block/sprout #8"],
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

test("parseSupportedLinkPreview parses generic HTTPS URLs", () => {
  assert.deepEqual(
    parseSupportedLinkPreview("https://example.com/articles/rich-previews"),
    {
      kind: "generic-link",
      href: "https://example.com/articles/rich-previews",
      provider: "example.com",
      title: "example.com",
      typeLabel: "link",
    },
  );
});

test("parseSupportedLinkPreview rejects generic HTTP URLs", () => {
  assert.equal(
    parseSupportedLinkPreview("http://example.com/articles/rich-previews"),
    null,
  );
});

test("extractSupportedLinkPreviews finds generic links and preserves exclusions", () => {
  assert.deepEqual(
    extractSupportedLinkPreviews(
      [
        "Read https://example.com/article first.",
        "`https://hidden.example.com/secret`",
        "then [the details](https://docs.example.org/details)",
      ].join(" "),
    ).map(({ kind, title }) => ({ kind, title })),
    [
      { kind: "generic-link", title: "example.com" },
      { kind: "generic-link", title: "the details" },
    ],
  );
});
