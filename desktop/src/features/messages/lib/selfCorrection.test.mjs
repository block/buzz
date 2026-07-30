import assert from "node:assert/strict";
import test from "node:test";

import { applySelfCorrection, parseSelfCorrection } from "./selfCorrection.ts";

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

test("accepts alternate punctuation delimiters", () => {
  for (const cmd of ["s|a|b|", "s#a#b#", "s,a,b,", "s:a:b:"]) {
    assert.deepEqual(
      parseSelfCorrection(cmd),
      { pattern: "a", replacement: "b", global: false, ignoreCase: false },
      cmd,
    );
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
    "s3://bucket/key", // alphanumeric delimiter → not a command
    "she said s/x/y/", // does not start with the command
    "hello world",
    "s a b", // whitespace delimiter → not a command
    "s\\a\\b\\", // backslash delimiter → not a command
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
