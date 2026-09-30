import assert from "node:assert/strict";
import test from "node:test";

import {
  idFromLabel,
  buildEnvRecord,
  filterArgs,
  envPairsFromRecord,
  formValuesFromCatalogEntry,
  definitionFromFormValues,
  commaArgError,
} from "./harnessFormLogic.ts";

// ── idFromLabel ──────────────────────────────────────────────────────────────

test("idFromLabel_typicalName_producesHyphenated", () => {
  assert.equal(idFromLabel("My Runtime"), "my-runtime");
});

test("idFromLabel_alreadyLowercase_returnsUnchanged", () => {
  assert.equal(idFromLabel("cursor"), "cursor");
});

test("idFromLabel_specialChars_replacedWithHyphens", () => {
  assert.equal(idFromLabel("Foo Bar!Baz"), "foo-bar-baz");
});

test("idFromLabel_consecutiveSpecialChars_collapsedToOneHyphen", () => {
  assert.equal(idFromLabel("foo   bar"), "foo-bar");
});

test("idFromLabel_trailingSpecialChars_stripped", () => {
  assert.equal(idFromLabel("my-runtime--"), "my-runtime");
});

test("idFromLabel_leadingHyphen_stripped", () => {
  // A label starting with a non-[a-z0-9_] char produces a leading hyphen;
  // the regex strips it so the id stays valid.
  assert.equal(idFromLabel("-bad"), "bad");
});

test("idFromLabel_underscorePreserved", () => {
  assert.equal(idFromLabel("my_agent"), "my_agent");
});

test("idFromLabel_emptyString_returnsEmpty", () => {
  assert.equal(idFromLabel(""), "");
});

test("idFromLabel_uppercaseOnly_lowercased", () => {
  assert.equal(idFromLabel("CURSOR"), "cursor");
});

// ── buildEnvRecord ───────────────────────────────────────────────────────────

test("buildEnvRecord_twoValidPairs_returnsBothEntries", () => {
  assert.deepEqual(
    buildEnvRecord([
      { key: "FOO", value: "bar" },
      { key: "BAZ", value: "qux" },
    ]),
    { FOO: "bar", BAZ: "qux" },
  );
});

test("buildEnvRecord_emptyKey_skipped", () => {
  assert.deepEqual(
    buildEnvRecord([
      { key: "", value: "orphaned-value" },
      { key: "KEEP", value: "yes" },
    ]),
    { KEEP: "yes" },
  );
});

test("buildEnvRecord_whitespaceOnlyKey_skipped", () => {
  assert.deepEqual(buildEnvRecord([{ key: "   ", value: "x" }]), {});
});

test("buildEnvRecord_keyWithLeadingTrailingSpaces_trimmed", () => {
  // Key is trimmed; value is preserved verbatim.
  assert.deepEqual(buildEnvRecord([{ key: "  MY_VAR  ", value: " val " }]), {
    MY_VAR: " val ",
  });
});

test("buildEnvRecord_emptyPairs_returnsEmptyObject", () => {
  assert.deepEqual(buildEnvRecord([]), {});
});

test("buildEnvRecord_duplicateKeys_lastValueWins", () => {
  assert.deepEqual(
    buildEnvRecord([
      { key: "X", value: "first" },
      { key: "X", value: "second" },
    ]),
    { X: "second" },
  );
});

// ── filterArgs ───────────────────────────────────────────────────────────────

test("filterArgs_normalArgs_allPreserved", () => {
  assert.deepEqual(filterArgs(["--flag", "--output", "file.txt"]), [
    "--flag",
    "--output",
    "file.txt",
  ]);
});

test("filterArgs_emptyString_removed", () => {
  assert.deepEqual(filterArgs([""]), []);
});

test("filterArgs_whitespaceOnlyArg_removed", () => {
  assert.deepEqual(filterArgs(["  "]), []);
});

test("filterArgs_mixedEmptyAndReal_onlyRealKept", () => {
  assert.deepEqual(filterArgs(["acp", "", "--verbose", "   "]), [
    "acp",
    "--verbose",
  ]);
});

test("filterArgs_argWithInternalSpaces_preserved", () => {
  // An arg with leading/trailing spaces but non-whitespace content is kept
  // (trim only tests emptiness, not the full value).
  assert.deepEqual(filterArgs(["--name", "hello world"]), [
    "--name",
    "hello world",
  ]);
});

test("filterArgs_emptyArray_returnsEmpty", () => {
  assert.deepEqual(filterArgs([]), []);
});

// ── envPairsFromRecord ────────────────────────────────────────────────────────

test("envPairsFromRecord_undefinedRecord_returnsEmptyArray", () => {
  assert.deepEqual(envPairsFromRecord(undefined), []);
});

test("envPairsFromRecord_emptyRecord_returnsEmptyArray", () => {
  assert.deepEqual(envPairsFromRecord({}), []);
});

test("envPairsFromRecord_singleEntry_returnsSinglePair", () => {
  assert.deepEqual(envPairsFromRecord({ FOO: "bar" }), [
    { key: "FOO", value: "bar" },
  ]);
});

test("envPairsFromRecord_multipleEntries_returnsAllPairs", () => {
  const result = envPairsFromRecord({ ALPHA: "1", BETA: "2" });
  // BTreeMap serializes in sorted key order; verify both pairs are present.
  assert.equal(result.length, 2);
  assert.ok(result.some((p) => p.key === "ALPHA" && p.value === "1"));
  assert.ok(result.some((p) => p.key === "BETA" && p.value === "2"));
});

test("envPairsFromRecord_valueCanBeEmptyString", () => {
  assert.deepEqual(envPairsFromRecord({ EMPTY_VAL: "" }), [
    { key: "EMPTY_VAL", value: "" },
  ]);
});

test("envPairsFromRecord_roundTrip_buildEnvRecord_restoresOriginal", () => {
  // Proves the edit round-trip: catalog env → pairs → save payload → same map.
  const original = { FOO: "bar", BAZ: "qux" };
  const pairs = envPairsFromRecord(original);
  const restored = buildEnvRecord(pairs);
  assert.deepEqual(restored, original);
});

// ── edit round-trip (entry → form → definition) ──────────────────────────────

/** A fully-populated definition as it appears on a catalog entry. */
const FULL_ENTRY = {
  id: "my-harness",
  label: "My Harness",
  command: "my-harness",
  defaultArgs: ["acp", "--verbose"],
  definitionEnv: { FOO: "bar", BAZ: "qux" },
  installInstructionsUrl: "https://example.com/docs",
  installHint: "npm install -g my-harness",
};

test("formValuesFromCatalogEntry_fullEntry_populatesEveryField", () => {
  const form = formValuesFromCatalogEntry(FULL_ENTRY);
  assert.equal(form.id, "my-harness");
  assert.equal(form.label, "My Harness");
  assert.equal(form.command, "my-harness");
  assert.deepEqual(form.args, ["acp", "--verbose"]);
  assert.deepEqual(form.env, [
    { key: "FOO", value: "bar" },
    { key: "BAZ", value: "qux" },
  ]);
  assert.equal(form.installInstructionsUrl, "https://example.com/docs");
  assert.equal(form.installHint, "npm install -g my-harness");
});

test("editRoundTrip_openThenSaveWithoutChanges_losesNothing", () => {
  // Regression for the installHint edit-loss bug: opening the edit form and
  // immediately saving must reproduce the original definition verbatim.
  const definition = definitionFromFormValues(
    formValuesFromCatalogEntry(FULL_ENTRY),
  );
  assert.deepEqual(definition, {
    id: FULL_ENTRY.id,
    label: FULL_ENTRY.label,
    command: FULL_ENTRY.command,
    args: FULL_ENTRY.defaultArgs,
    env: FULL_ENTRY.definitionEnv,
    installInstructionsUrl: FULL_ENTRY.installInstructionsUrl,
    installHint: FULL_ENTRY.installHint,
  });
});

test("editRoundTrip_definitionKeys_matchFormSeedFields", () => {
  // Structural guard: any user-editable field added to the save payload must
  // also be seeded on edit, or open-edit-then-save silently erases it (the
  // installHint bug). `args`/`env` map to `defaultArgs`/`definitionEnv`.
  const seeded = new Set(Object.keys(formValuesFromCatalogEntry(FULL_ENTRY)));
  const saved = Object.keys(
    definitionFromFormValues(formValuesFromCatalogEntry(FULL_ENTRY)),
  );
  for (const key of saved) {
    assert.ok(
      seeded.has(key),
      `save payload field "${key}" not seeded on edit`,
    );
  }
});

test("formValuesFromCatalogEntry_nullishOptionalFields_defaultsSafely", () => {
  const form = formValuesFromCatalogEntry({
    id: "min",
    label: "Min",
    command: null,
    defaultArgs: undefined,
    definitionEnv: undefined,
    installInstructionsUrl: "",
    installHint: "",
  });
  assert.equal(form.command, "");
  assert.deepEqual(form.args, []);
  assert.deepEqual(form.env, []);
});

test("definitionFromFormValues_trimsScalarFields", () => {
  const definition = definitionFromFormValues({
    id: " my-id ",
    label: " Label ",
    command: " cmd ",
    args: ["keep", "  "],
    env: [{ key: " K ", value: "v" }],
    installInstructionsUrl: " https://x ",
    installHint: " hint ",
  });
  assert.equal(definition.id, "my-id");
  assert.equal(definition.label, "Label");
  assert.equal(definition.command, "cmd");
  assert.deepEqual(definition.args, ["keep"]);
  assert.deepEqual(definition.env, { K: "v" });
  assert.equal(definition.installInstructionsUrl, "https://x");
  assert.equal(definition.installHint, "hint");
});

// ── commaArgError ─────────────────────────────────────────────────────────────

test("commaArgError_noCommas_returnsNull", () => {
  assert.equal(commaArgError(["acp", "--verbose", "--name", "x y"]), null);
});

test("commaArgError_emptyArgs_returnsNull", () => {
  assert.equal(commaArgError([]), null);
});

test("commaArgError_commaInArg_namesOffender", () => {
  const err = commaArgError(["--models", "a,b"]);
  assert.ok(err !== null);
  assert.ok(err.includes('"a,b"'), "error must name the offending argument");
  assert.ok(err.includes("comma"), "error must name the transport limit");
});

test("commaArgError_firstOffenderReported", () => {
  const err = commaArgError(["x,y", "p,q"]);
  assert.ok(err?.includes('"x,y"'));
});
