import assert from "node:assert/strict";
import test from "node:test";

import { personaDTag, slugFromName } from "./spawnerPreference.ts";

// The Rust side (`check_spec_slug` in buzz-sdk) accepts only lowercase ASCII
// letters, digits, hyphens, and underscores, must not start with `-` or `_`,
// and caps at 64 bytes. Slugs reach container names, volume names, and log
// paths on the spawner host, so anything else is rejected outright. These
// tests pin the normalization that keeps a user from hitting that wall.
const ACCEPTED = /^[a-z0-9][a-z0-9_-]*$/;

test("lowercasesAndHyphenatesOrdinaryNames", () => {
  assert.equal(slugFromName("Fizz"), "fizz");
  assert.equal(slugFromName("Code Reviewer"), "code-reviewer");
});

test("stripsCharactersTheSpawnerWouldReject", () => {
  // "Fizz (prod)" would otherwise fail validation with a message the user
  // cannot act on.
  const slug = slugFromName("Fizz (prod)!");
  assert.equal(slug, "fizz-prod");
  assert.match(slug, ACCEPTED);
});

test("neverLeadsWithAHyphenOrUnderscore", () => {
  // A leading hyphen is explicitly rejected by check_spec_slug.
  assert.equal(slugFromName("  -Fizz"), "fizz");
  assert.equal(slugFromName("!!!Fizz"), "fizz");
  assert.match(slugFromName("-_-Fizz"), ACCEPTED);
});

test("neverTrailsWithAHyphenEvenAfterTruncation", () => {
  // Truncating at 64 bytes can land mid-separator and reintroduce a trailing
  // hyphen after the first strip.
  const slug = slugFromName(`${"a".repeat(63)} tail`);
  assert.ok(slug.length <= 64);
  assert.ok(!slug.endsWith("-"));
  assert.match(slug, ACCEPTED);
});

test("respectsTheSixtyFourByteCap", () => {
  const slug = slugFromName("x".repeat(200));
  assert.equal(slug.length, 64);
  assert.match(slug, ACCEPTED);
});

test("returnsNullWhenNothingUsableSurvives", () => {
  // Better to refuse with a clear message than to publish an empty d-tag the
  // spawner cannot address.
  assert.equal(slugFromName(""), null);
  assert.equal(slugFromName("   "), null);
  assert.equal(slugFromName("!!!"), null);
  assert.equal(slugFromName("日本語"), null);
});

test("mapsAPersonaIdToItsRelayDTag", () => {
  // The relay rejects a `d` tag containing a colon, so the built-ins are
  // published under a normalised slug. A spec carrying the raw id would point
  // at a persona that cannot exist.
  assert.equal(personaDTag("builtin:fizz"), "builtin-fizz");
  assert.equal(personaDTag("builtin:honey"), "builtin-honey");
  assert.equal(personaDTag("Code Reviewer"), "code-reviewer");
});

test("personaDTagAlwaysStartsAlphanumericAndFitsTheGrammar", () => {
  const accepted = /^[a-z0-9][a-z0-9_-]{0,63}$/;
  assert.match(personaDTag(":leading-colon"), accepted);
  assert.match(personaDTag("_underscore"), accepted);
  assert.match(personaDTag("x".repeat(200)), accepted);
});
