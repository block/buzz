import assert from "node:assert/strict";
import test from "node:test";

import {
  applySelfCorrection,
  buildSelfCorrectionEdit,
  parseSelfCorrection,
} from "./selfCorrection.ts";

// ── parseSelfCorrection ────────────────────────────────────────────────────

test("parses a basic s/old/new/ command", () => {
  assert.deepEqual(parseSelfCorrection("s/hullo/hello/"), {
    pattern: "hullo",
    replacement: "hello",
    global: false,
    ignoreCase: false,
  });
});

test("trailing delimiter is optional when there are no flags", () => {
  assert.deepEqual(parseSelfCorrection("s/hullo/hello"), {
    pattern: "hullo",
    replacement: "hello",
    global: false,
    ignoreCase: false,
  });
});

test("empty replacement is a valid deletion", () => {
  assert.deepEqual(parseSelfCorrection("s/oops//"), {
    pattern: "oops",
    replacement: "",
    global: false,
    ignoreCase: false,
  });
});

test("only the canonical slash delimiter is a command", () => {
  // Alternate punctuation delimiters are intentionally NOT commands — they widen
  // the accidental-trigger surface for no gain (`\/` escaping already covers a
  // slash inside the pattern). Slash-only matches the IRC `s/old/new/` shorthand.
  for (const cmd of ["s|a|b|", "s#a#b#", "s,a,b,", "s:a:b:", "s~a~b~"]) {
    assert.equal(parseSelfCorrection(cmd), null, cmd);
  }
});

test("parses g and i flags in any order", () => {
  assert.deepEqual(parseSelfCorrection("s/a/b/g"), {
    pattern: "a",
    replacement: "b",
    global: true,
    ignoreCase: false,
  });
  assert.deepEqual(parseSelfCorrection("s/a/b/gi"), {
    pattern: "a",
    replacement: "b",
    global: true,
    ignoreCase: true,
  });
  assert.deepEqual(parseSelfCorrection("s/a/b/ig"), {
    pattern: "a",
    replacement: "b",
    global: true,
    ignoreCase: true,
  });
});

test("honours escaped delimiters and backslashes inside sections", () => {
  assert.deepEqual(parseSelfCorrection("s/a\\/b/c\\/d/"), {
    pattern: "a/b",
    replacement: "c/d",
    global: false,
    ignoreCase: false,
  });
  assert.deepEqual(parseSelfCorrection("s/a\\\\b/c/"), {
    pattern: "a\\b",
    replacement: "c",
    global: false,
    ignoreCase: false,
  });
});

test("non-delimiter backslashes are preserved verbatim", () => {
  assert.deepEqual(parseSelfCorrection("s/a/c\\nd/"), {
    pattern: "a",
    replacement: "c\\nd",
    global: false,
    ignoreCase: false,
  });
});

test("returns null for non-commands so they send as literal text", () => {
  for (const input of [
    "",
    "s",
    "s/",
    "s//",
    "s/a", // no closing delimiter for the pattern
    "s///", // empty pattern
    "s3://bucket/key", // second char isn't the slash delimiter → not a command
    "she said s/x/y/", // does not start with the command
    "hello world",
    "s: notes for later", // non-slash char after `s` → ordinary prose, not a command
    "s|a|b|", // alternate delimiters are no longer commands
    "s a b", // whitespace after `s` → not a command
    "s\\a\\b\\", // backslash after `s` → not a command
    "s/a/b/x", // unknown flag
    "s/a/b/gg", // repeated flag
  ]) {
    assert.equal(parseSelfCorrection(input), null, JSON.stringify(input));
  }
});

// ── applySelfCorrection ────────────────────────────────────────────────────

const cmd = (overrides) => ({
  pattern: "a",
  replacement: "b",
  global: false,
  ignoreCase: false,
  ...overrides,
});

test("replaces the first occurrence by default", () => {
  assert.equal(
    applySelfCorrection("banana", cmd({ pattern: "a", replacement: "o" })),
    "bonana",
  );
});

test("global flag replaces every occurrence", () => {
  assert.equal(
    applySelfCorrection(
      "banana",
      cmd({ pattern: "a", replacement: "o", global: true }),
    ),
    "bonono",
  );
});

test("does not re-match inside the substituted text", () => {
  // Replacing "a" with "aa" globally must not loop forever or double up.
  assert.equal(
    applySelfCorrection(
      "aa",
      cmd({ pattern: "a", replacement: "aa", global: true }),
    ),
    "aaaa",
  );
});

test("ignoreCase matches case-insensitively but preserves surrounding text", () => {
  assert.equal(
    applySelfCorrection(
      "The HULLO there",
      cmd({ pattern: "hullo", replacement: "hello", ignoreCase: true }),
    ),
    "The hello there",
  );
});

test("returns null when the pattern is absent", () => {
  assert.equal(
    applySelfCorrection("hello", cmd({ pattern: "xyz", replacement: "q" })),
    null,
  );
});

test("deletion removes the matched text", () => {
  assert.equal(
    applySelfCorrection("hel lo", cmd({ pattern: " ", replacement: "" })),
    "hello",
  );
});

test("pattern and replacement are literal text, never regex", () => {
  // `s/*/x` replaces the first literal `*` with `x` — no regex metacharacters.
  assert.equal(
    applySelfCorrection("a*b*c", parseSelfCorrection("s/*/x")),
    "axb*c",
  );
  // `.` is a literal dot, not a wildcard: absent from "abc" → nothing to correct.
  assert.equal(applySelfCorrection("abc", parseSelfCorrection("s/./X/")), null);
  assert.equal(
    applySelfCorrection("a.c", parseSelfCorrection("s/./X/")),
    "aXc",
  );
  // Other regex-special chars match themselves in both pattern and replacement.
  assert.equal(
    applySelfCorrection("1+1", parseSelfCorrection("s/+/ plus /")),
    "1 plus 1",
  );
  assert.equal(
    applySelfCorrection("(a)", parseSelfCorrection("s/(a)/[b]/")),
    "[b]",
  );
  // Replacement is inserted verbatim — no `$1` backrefs or `\d` interpretation.
  assert.equal(
    applySelfCorrection("hi", parseSelfCorrection("s/hi/$1\\d/")),
    "$1\\d",
  );
});

test("end-to-end: parse then apply the motivating example", () => {
  const command = parseSelfCorrection("s/u/e/");
  assert.ok(command);
  assert.equal(applySelfCorrection("hullo there!", command), "hello there!");
});

test("end-to-end: trailing delimiter omitted — s/u/e", () => {
  const command = parseSelfCorrection("s/u/e");
  assert.ok(command);
  assert.equal(applySelfCorrection("hullo there!", command), "hello there!");
});

test("end-to-end: escaped delimiters — s/\\//\\/\\//g doubles every slash", () => {
  const command = parseSelfCorrection("s/\\//\\/\\//g");
  assert.deepEqual(command, {
    pattern: "/",
    replacement: "//",
    global: true,
    ignoreCase: false,
  });
  assert.equal(applySelfCorrection("a/b/c", command), "a//b//c");
});

// ── buildSelfCorrectionEdit ─────────────────────────────────────────────────

const target = (overrides) => ({ body: "", tags: [], ...overrides });

test("buildSelfCorrectionEdit: rebuilds the corrected body as an edit", () => {
  const edit = buildSelfCorrectionEdit(
    target({ body: "hullo there" }),
    parseSelfCorrection("s/hullo/hello/"),
    [],
  );
  assert.deepEqual(edit, { content: "hello there", tags: [] });
});

test("buildSelfCorrectionEdit: null when the pattern is absent from the target", () => {
  assert.equal(
    buildSelfCorrectionEdit(
      target({ body: "hello" }),
      parseSelfCorrection("s/xyz/q/"),
      [],
    ),
    null,
  );
});

test("buildSelfCorrectionEdit: re-attaches NIP-30 emoji tags for shortcodes in the corrected body", () => {
  const edit = buildSelfCorrectionEdit(
    target({ body: "hi :bee:" }),
    parseSelfCorrection("s/hi/yo/"),
    [{ shortcode: "bee", url: "https://cdn/bee.png" }],
  );
  assert.equal(edit?.content, "yo :bee:");
  assert.ok(
    edit?.tags.some((tag) => tag[0] === "emoji" && tag[1] === "bee"),
    "expected an emoji tag for :bee:",
  );
});
