import assert from "node:assert/strict";
import test from "node:test";

import { sanitizeChannelName, uniqueChannelName } from "./sessionNaming.ts";

test("sanitizeChannelName_buildsHyphenatedNameFromWords", () => {
  assert.equal(
    sanitizeChannelName("Fix the login race condition"),
    "fix-the-login-race-condition",
  );
});

test("sanitizeChannelName_stripsPunctuationAndNormalizesCase", () => {
  assert.equal(
    sanitizeChannelName("Why is CI broken?! (again)"),
    "why-is-ci-broken-again",
  );
});

test("sanitizeChannelName_capsLengthAtWordBoundary", () => {
  const name = sanitizeChannelName(
    "one two three four five six seven eight nine ten eleven twelve",
  );
  assert.ok(name.length <= 40, `expected <= 40 chars, got ${name.length}`);
  assert.ok(!name.endsWith("-"), "must not end mid-separator");
});

test("sanitizeChannelName_truncatesSingleOversizedWord", () => {
  assert.equal(sanitizeChannelName("a".repeat(100)), "a".repeat(40));
});

test("sanitizeChannelName_emptyForSymbolOnlyInput", () => {
  assert.equal(sanitizeChannelName(""), "");
  assert.equal(sanitizeChannelName("!!! ???"), "");
});

test("uniqueChannelName_returnsBaseWhenFree", () => {
  assert.equal(uniqueChannelName("fix-login", new Set()), "fix-login");
});

test("uniqueChannelName_suffixesUntilNameIsUnique", () => {
  const existing = new Set(["fix-login", "fix-login-2"]);
  assert.equal(uniqueChannelName("fix-login", existing), "fix-login-3");
});
