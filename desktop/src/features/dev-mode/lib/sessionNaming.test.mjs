import assert from "node:assert/strict";
import test from "node:test";

import { slugifyPrompt } from "./sessionNaming.ts";

const none = new Set();

test("slugifyPrompt_buildsHyphenatedNameFromPromptWords", () => {
  assert.equal(
    slugifyPrompt("Fix the login race condition", none),
    "fix-the-login-race-condition",
  );
});

test("slugifyPrompt_stripsPunctuationAndNormalizesCase", () => {
  assert.equal(
    slugifyPrompt("Why is CI broken?! (again)", none),
    "why-is-ci-broken-again",
  );
});

test("slugifyPrompt_capsLengthAtWordBoundary", () => {
  const slug = slugifyPrompt(
    "one two three four five six seven eight nine ten eleven twelve",
    none,
  );
  assert.ok(slug.length <= 40, `expected <= 40 chars, got ${slug.length}`);
  assert.ok(!slug.endsWith("-"), "must not end mid-separator");
});

test("slugifyPrompt_truncatesSingleOversizedWord", () => {
  const slug = slugifyPrompt("a".repeat(100), none);
  assert.equal(slug, "a".repeat(40));
});

test("slugifyPrompt_fallsBackForEmptyOrSymbolOnlyPrompts", () => {
  assert.equal(slugifyPrompt("", none), "session");
  assert.equal(slugifyPrompt("!!! ???", none), "session");
});

test("slugifyPrompt_suffixesUntilNameIsUnique", () => {
  const existing = new Set(["fix-login", "fix-login-2"]);
  assert.equal(slugifyPrompt("Fix login", existing), "fix-login-3");
});
