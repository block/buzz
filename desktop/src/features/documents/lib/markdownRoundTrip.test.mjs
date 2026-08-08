/**
 * The corpus test: real Obsidian constructs through the real TipTap pipeline.
 *
 * This is the regression net for the round-trip guard. Its job is not to prove
 * TipTap is lossless -- it demonstrably is not -- but to pin down *which*
 * constructs survive, so that a future extension that changes the answer fails
 * here loudly instead of silently rewriting someone's vault.
 *
 * Runs under jsdom because a ProseMirror editor needs a DOM.
 */
import assert from "node:assert/strict";
import { before, test } from "node:test";
import { JSDOM } from "jsdom";

import { splitFrontmatter } from "./frontmatter.ts";
import { isRoundTripStable } from "./roundTripGuard.ts";

let reserializeMarkdown;
let destroyMarkdownProbe;

before(async () => {
  const dom = new JSDOM("<!doctype html><html><body></body></html>");
  globalThis.window = dom.window;
  globalThis.document = dom.window.document;
  globalThis.HTMLElement = dom.window.HTMLElement;
  globalThis.Element = dom.window.Element;
  globalThis.Node = dom.window.Node;
  globalThis.DocumentFragment = dom.window.DocumentFragment;
  globalThis.getComputedStyle = dom.window.getComputedStyle;
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
  });

  ({ destroyMarkdownProbe, reserializeMarkdown } = await import(
    "./markdownRoundTrip.ts"
  ));
});

/**
 * Constructs the editor must not touch. A regression here is a real bug.
 *
 * The Obsidian entries survive because they are plain text to this schema and
 * `toDiskMarkdown` undoes the serializer's escaping of `[`, `]`, `~` and `_`.
 * That is exactly why the escape-stripping exists.
 */
const MUST_BE_STABLE = [
  ["heading and paragraph", "# Title\n\nA paragraph."],
  ["emphasis and strong", "Some *emphasis* and **bold**."],
  ["inline code", "Call `useVaultEditor()` first."],
  ["fenced code with language", "```js\nconst a = 1;\n```"],
  ["fenced code containing dashes", "```\n---\nnot frontmatter\n---\n```"],
  ["bullet list", "- one\n- two"],
  ["star bullet marker", "* item"],
  ["plus bullet marker", "+ item"],
  ["soft-wrapped blockquote", "> Prose that wraps\n> across two lines."],
  ["nested bullet list", "- one\n- two\n  - nested"],
  ["ordered list", "1. first\n2. second"],
  ["task list", "- [ ] todo\n- [x] done"],
  ["blockquote", "> quoted text"],
  // These two pin the exceptions in `normalizeBlockSeparation`. A blank line
  // between same-family blocks is content -- it makes a list loose, and it is
  // the only thing keeping two quotes from merging. Both survive the editor
  // intact, so collapsing them as "block separation" would hide a real change.
  ["loose list", "- one\n\n- two"],
  ["adjacent blockquotes", "> first\n\n> second"],
  ["link", "See [the docs](https://example.com)."],
  ["thematic break", "before\n\n---\n\nafter"],
  ["gfm table", "| a | b |\n| --- | --- |\n| 1 | 2 |"],
  [
    "gfm table with hand-aligned delimiters",
    "| Version | Supported |\n| ------- | --------- |\n| main | Active |",
  ],
  ["multiple paragraphs", "one\n\ntwo\n\nthree"],
  ["strikethrough", "~~gone~~"],
  // Differences a reader cannot see. Each of these was measured against a real
  // 470-note vault; together they took the pass rate from 4% to 63%. They are
  // tolerated by `normalizeForComparison`, not by the editor -- saving still
  // rewrites them into canonical form, which is the accepted trade.
  ["underscore emphasis", "_emphasis_"],
  ["underscore strong", "__strong__"],
  ["intraword underscores", "_See weekly_report.py for details._"],
  ["snake_case left alone", "Call load_user_profile() first."],
  ["single trailing space", "A line with one trailing space. "],
  ["list directly under a heading", "## Today\n- one\n- two"],
  ["paragraph directly under a heading", "## Today\nSome prose."],
  ["several blank lines between paragraphs", "one\n\n\n\ntwo"],
  ["star thematic break", "before\n\n***\n\nafter"],
  ["underscore thematic break", "before\n\n___\n\nafter"],
  ["unpadded table", "|a|b|\n|---|---|\n|1|2|"],
  ["double spaces inside a line", "Two  spaces  between  words."],
  ["wikilink", "A [[Note Title]] reference."],
  ["wikilink with alias", "See [[Note|the note]]."],
  ["embed", "![[Some Note]]"],
  ["block reference", "A claim. ^block-id"],
  ["tag", "A #tag here."],
  ["highlight", "==highlight=="],
  ["comment", "%%comment%%"],
  ["math", "$$x^2$$"],
];

/**
 * Constructs the editor mangles. These are the reason the guard exists: each
 * must be *detected* so the file opens in source mode, never silently
 * rewritten.
 *
 * Tables used to head this list — without the table extensions a GFM table
 * serialized down to its concatenated cell text (`| a | b |…` became `ab12`).
 * They now round-trip, and moved to MUST_BE_STABLE.
 */
const MUST_BE_DETECTED_LOSSY = [
  ["callout", "> [!info] Title\n> body"],
  ["footnote", "Text[^1]\n\n[^1]: note"],
  ["raw html", "<div>raw</div>"],
  ["setext heading", "Title\n====="],
  ["four-space nesting", "- a\n    - b"],
  ["two-space hard break", "line one  \nline two"],
  ["repeated ordered marker", "1. a\n1. b"],
];

test("stable constructs survive the editor untouched", () => {
  for (const [label, source] of MUST_BE_STABLE) {
    assert.equal(
      isRoundTripStable(source, reserializeMarkdown),
      true,
      `${label} should round-trip cleanly but did not.\n` +
        `  in:  ${JSON.stringify(source)}\n` +
        `  out: ${JSON.stringify(reserializeMarkdown(source))}`,
    );
  }
});

test("lossy constructs are detected rather than silently rewritten", () => {
  for (const [label, source] of MUST_BE_DETECTED_LOSSY) {
    assert.equal(
      isRoundTripStable(source, reserializeMarkdown),
      false,
      `${label} round-tripped cleanly — if an extension now handles it, move ` +
        `it into MUST_BE_STABLE.`,
    );
  }
});

test("frontmatter is destroyed by the editor, which is why we split it off", () => {
  // Pinning the exact failure mode: the opening `---` becomes a thematic break
  // and the YAML becomes a heading. This is the single largest corruption
  // source in a real vault.
  const raw = "---\ntitle: Note\n---\n\n# Body";
  assert.equal(
    isRoundTripStable(raw, reserializeMarkdown),
    false,
    "raw frontmatter must not be considered safe to live-edit",
  );

  // Split first, and the body alone is perfectly safe.
  const { body, frontmatter } = splitFrontmatter(raw);
  assert.equal(frontmatter, "---\ntitle: Note\n---\n\n");
  assert.equal(
    isRoundTripStable(body, reserializeMarkdown),
    true,
    "the body below frontmatter should round-trip cleanly",
  );
});

test("a realistic note with frontmatter and prose is editable after splitting", () => {
  const raw = [
    "---",
    "title: Meeting notes",
    "tags: [work, buzz]",
    "---",
    "",
    "# Meeting notes",
    "",
    "- Ship Documents",
    "- Then wikilinks",
    "",
    "Some **bold** and a [link](https://example.com).",
  ].join("\n");

  const { body } = splitFrontmatter(raw);
  assert.equal(isRoundTripStable(body, reserializeMarkdown), true);
});

test("the probe can be destroyed and lazily rebuilt", () => {
  destroyMarkdownProbe();
  assert.equal(isRoundTripStable("# Title", reserializeMarkdown), true);
  destroyMarkdownProbe();
});
