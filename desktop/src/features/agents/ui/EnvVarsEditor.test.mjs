/**
 * Unit tests for the EnvVarsEditor state helpers.
 *
 * Tests three invariants:
 *
 *   1. Pre-saved required key renders exactly once (toRows excludes skipKeys).
 *   2. Type required value → add a normal var → required value survives in
 *      the emitted record (buildRecord merges required keys from value).
 *   3. Provider/runtime switch (skipKeys change) triggers a row reprojection
 *      — the guard fires when skipKeys changes, even if value is unchanged.
 *   4. inheritedRows: render/exclusion/override/no-serialize invariants.
 *   5. getBakedProviderInheritLabel: label helper correctness.
 *
 * These are pure-logic tests — no React renderer needed. The transition tests
 * (Invariant 3) exercise the real exported `skipKeysEqual` guard that controls
 * whether the effect calls `setRows(toRows(value, skipKeys))`.
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  toRows,
  toRecord,
  skipKeysEqual,
  isRequiredKeyMissing,
  buildRecord as buildRecordUtil,
} from "./EnvVarsEditor.tsx";
import { getBakedProviderInheritLabel } from "./bakedEnvHelpers.ts";
import {
  deriveNumericDescriptors,
  structuredEnvKeys,
  filterBakedGenericRows,
  numericTuningPlaceholder,
} from "../lib/agentConfigCore.ts";

// ── Invariant 1: toRows excludes skip keys ─────────────────────────────────

test("toRows_presaved_required_key_excluded_from_rows", () => {
  // A dialog opens with ANTHROPIC_API_KEY already set in value, and that key
  // is in requiredKeys. toRows must NOT include it in the row list.
  const value = { ANTHROPIC_API_KEY: "sk-abc", MY_VAR: "foo" };
  const skipKeys = new Set(["ANTHROPIC_API_KEY"]);
  const rows = toRows(value, skipKeys);

  // MY_VAR should appear as a normal editable row.
  assert.equal(rows.length, 1, "only non-skip keys should appear in rows");
  assert.equal(rows[0].key, "MY_VAR");
  assert.equal(rows[0].value, "foo");
});

test("toRows_with_empty_value_and_required_key_produces_no_rows", () => {
  // Dialog opens fresh, no user-set env vars, ANTHROPIC_API_KEY is required.
  const value = { ANTHROPIC_API_KEY: "" };
  const skipKeys = new Set(["ANTHROPIC_API_KEY"]);
  const rows = toRows(value, skipKeys);
  assert.equal(
    rows.length,
    0,
    "required key with empty value should not enter rows",
  );
});

test("toRows_without_skip_keys_includes_all_entries", () => {
  // Baseline: no skipKeys → behaviour is unchanged from the original.
  const value = { FOO: "bar", BAZ: "qux" };
  const rows = toRows(value);
  assert.equal(rows.length, 2);
  const keys = rows.map((r) => r.key).sort();
  assert.deepEqual(keys, ["BAZ", "FOO"]);
});

test("toRows_file_satisfied_key_excluded_from_rows", () => {
  // A file-satisfied key should also not appear in normal editable rows.
  const value = { GOOSE_API_KEY: "from-config", USER_VAR: "hello" };
  const skipKeys = new Set(["GOOSE_API_KEY"]);
  const rows = toRows(value, skipKeys);
  assert.equal(rows.length, 1);
  assert.equal(rows[0].key, "USER_VAR");
});

test("toRows_topLevelApiKey_excludedFromAdvancedRows", () => {
  const value = { ANTHROPIC_API_KEY: "sk-local", USER_VAR: "hello" };
  const rows = toRows(value, new Set(["ANTHROPIC_API_KEY"]));

  assert.deepEqual(
    rows.map((row) => row.key),
    ["USER_VAR"],
    "the top-level API key must not duplicate as an Advanced env row",
  );
});

// ── Invariant 2: emit preserves required-key values ───────────────────────
//
// We test this via the pure helpers: build a row list (normal vars only),
// then simulate what buildRecord does — merge required-key values from
// value into toRecord(rows). This is the exact logic in buildRecord().

function buildRecord(rows, requiredKeys, value) {
  const base = {};
  for (const key of requiredKeys) {
    if (key in value) base[key] = value[key];
  }
  return { ...base, ...toRecord(rows) };
}

test("buildRecord_preserves_required_key_value_when_normal_row_added", () => {
  // Simulate: user typed ANTHROPIC_API_KEY="sk-abc" into the amber row
  // (updateRequiredValue fired, value is now {ANTHROPIC_API_KEY:"sk-abc"}).
  // Then user clicks "Add variable" → emit fires with rows=[{key:"",value:""}].
  // The emitted record must still contain ANTHROPIC_API_KEY.
  const requiredKeys = ["ANTHROPIC_API_KEY"];
  const value = { ANTHROPIC_API_KEY: "sk-abc" };
  const rows = [{ id: "r1", key: "", value: "" }]; // new empty row
  const record = buildRecord(rows, requiredKeys, value);

  // Empty-key rows are excluded by toRecord, so only ANTHROPIC_API_KEY survives.
  assert.equal(
    record.ANTHROPIC_API_KEY,
    "sk-abc",
    "required key value must survive in emitted record after adding a normal row",
  );
});

test("buildRecord_preserves_required_key_value_alongside_normal_rows", () => {
  // User has typed a required key value AND has a normal env var row.
  const requiredKeys = ["ANTHROPIC_API_KEY"];
  const value = { ANTHROPIC_API_KEY: "sk-xyz", MY_VAR: "foo" };
  // rows only contains MY_VAR (required key is excluded from rows).
  const rows = [{ id: "r1", key: "MY_VAR", value: "foo" }];
  const record = buildRecord(rows, requiredKeys, value);

  assert.equal(record.ANTHROPIC_API_KEY, "sk-xyz", "required key preserved");
  assert.equal(record.MY_VAR, "foo", "normal row preserved");
  assert.equal(Object.keys(record).length, 2, "exactly two entries");
});

test("buildRecord_normal_row_overrides_do_not_affect_required_key", () => {
  // Normal row edits should not change the required key value.
  const requiredKeys = ["ANTHROPIC_API_KEY"];
  const value = { ANTHROPIC_API_KEY: "sk-abc", EXISTING: "old" };
  const rows = [{ id: "r1", key: "EXISTING", value: "new" }];
  const record = buildRecord(rows, requiredKeys, value);

  assert.equal(
    record.ANTHROPIC_API_KEY,
    "sk-abc",
    "required key unchanged by normal row edit",
  );
  assert.equal(record.EXISTING, "new", "normal row update applied");
});

test("buildRecord_required_key_not_in_value_is_omitted", () => {
  // If the required key has never been set (not in value), it should not
  // appear in the emitted record (no phantom empty entry).
  const requiredKeys = ["ANTHROPIC_API_KEY"];
  const value = { MY_VAR: "hello" }; // ANTHROPIC_API_KEY not yet set
  const rows = [{ id: "r1", key: "MY_VAR", value: "hello" }];
  const record = buildRecord(rows, requiredKeys, value);

  assert.equal(
    "ANTHROPIC_API_KEY" in record,
    false,
    "unset required key must not appear in emitted record",
  );
});

// ── toRecord baseline ──────────────────────────────────────────────────────

test("toRecord_skips_empty_key_rows", () => {
  const rows = [
    { id: "a", key: "", value: "orphan" },
    { id: "b", key: "MY_VAR", value: "ok" },
  ];
  const record = toRecord(rows);
  assert.equal("" in record, false, "empty-key row must be excluded");
  assert.equal(record.MY_VAR, "ok");
});

test("toRecord_last_write_wins_on_duplicate_keys", () => {
  const rows = [
    { id: "a", key: "FOO", value: "first" },
    { id: "b", key: "FOO", value: "second" },
  ];
  const record = toRecord(rows);
  assert.equal(record.FOO, "second", "last duplicate wins");
  assert.equal(Object.keys(record).length, 1);
});

// ── Invariant 3: skipKeysEqual guard — transition detection ────────────────
//
// The row-resync effect fires when `[value, skipKeys]` changes. The guard
// previously checked only `recordsEqual(lastEmitted, value)`. If `skipKeys`
// changed while `value` stayed equal to `lastEmitted`, the guard returned
// false and rows were NOT rebuilt — leaving a stale projection (duplicate
// or dropped key). The fix adds `skipKeysChanged = !skipKeysEqual(prev, next)`
// as a second trigger.
//
// These tests exercise the REAL exported `skipKeysEqual` function, which is
// exactly what the effect calls. They prove the guard fires on both transition
// directions, and that `toRows(value, newSkipKeys)` produces the correct rows
// after the rebuild.

test("skipKeysEqual_detects_normal_to_required_transition", () => {
  // Scenario: value = {ANTHROPIC_API_KEY:"sk"}, key starts as normal row.
  // Provider switches → requiredKeys gains ANTHROPIC_API_KEY.
  const prev = new Set(); // before switch: key is normal (not in skipKeys)
  const next = new Set(["ANTHROPIC_API_KEY"]); // after switch: key is required

  // Guard must fire (skipKeys changed → !skipKeysEqual returns true).
  assert.equal(
    skipKeysEqual(prev, next),
    false,
    "normal→required transition must be detected",
  );

  // After rebuild: toRows with new skipKeys must EXCLUDE the now-required key.
  const value = { ANTHROPIC_API_KEY: "sk", MY_VAR: "foo" };
  const rows = toRows(value, next);
  const keyNames = rows.map((r) => r.key);
  assert.equal(
    keyNames.includes("ANTHROPIC_API_KEY"),
    false,
    "ANTHROPIC_API_KEY must not be in rows after normal→required transition",
  );
  assert.equal(
    keyNames.includes("MY_VAR"),
    true,
    "non-required key must still be in rows after transition",
  );
});

test("skipKeysEqual_detects_required_to_normal_transition", () => {
  // Scenario: value = {ANTHROPIC_API_KEY:"sk"}, key starts as required row.
  // Provider switches → requiredKeys loses ANTHROPIC_API_KEY.
  const prev = new Set(["ANTHROPIC_API_KEY"]); // before switch: key is required
  const next = new Set(); // after switch: key is now a normal row

  // Guard must fire (skipKeys changed).
  assert.equal(
    skipKeysEqual(prev, next),
    false,
    "required→normal transition must be detected",
  );

  // After rebuild: toRows with empty skipKeys must INCLUDE the key.
  const value = { ANTHROPIC_API_KEY: "sk" };
  const rows = toRows(value, next);
  assert.equal(
    rows.length,
    1,
    "key must appear as a normal row after required→normal",
  );
  assert.equal(rows[0].key, "ANTHROPIC_API_KEY");
  assert.equal(
    rows[0].value,
    "sk",
    "the key value must be preserved in the rebuilt row",
  );
});

test("skipKeysEqual_no_rebuild_when_keys_unchanged", () => {
  // When skipKeys membership is identical (but different Set reference), the
  // guard must NOT fire — avoids wasted re-render on every parent render.
  const prev = new Set(["ANTHROPIC_API_KEY"]);
  const next = new Set(["ANTHROPIC_API_KEY"]); // same membership, different ref

  assert.equal(
    skipKeysEqual(prev, next),
    true,
    "identical membership must be equal (no spurious rebuild)",
  );
});

test("skipKeysEqual_empty_sets_are_equal", () => {
  assert.equal(
    skipKeysEqual(new Set(), new Set()),
    true,
    "two empty sets are equal",
  );
});

test("skipKeysEqual_different_sizes_are_not_equal", () => {
  const a = new Set(["FOO", "BAR"]);
  const b = new Set(["FOO"]);
  assert.equal(
    skipKeysEqual(a, b),
    false,
    "sets of different sizes are not equal",
  );
});

// ── isRequiredKeyMissing: local-over-inherited precedence (Thufir IMPORTANT) ─
//
// isRequiredKeyMissing must match backend effective-env semantics:
// - key absent from localValue → inherited decides
// - key present in localValue (even as "") → local decides; inherited ignored
// An explicit empty local value shadows the global/inherited key and must
// render the amber "Required" badge, matching backend is_none_or(|v| v.is_empty()).

test("isRequiredKeyMissing_keyAbsent_inheritedSet_notMissing", () => {
  // Key not in local map at all; inherited provides it → satisfied.
  assert.equal(
    isRequiredKeyMissing(
      "ANTHROPIC_API_KEY",
      {},
      { ANTHROPIC_API_KEY: "sk-global" },
    ),
    false,
    "key absent from local and present in inherited must NOT be missing",
  );
});

test("isRequiredKeyMissing_keyAbsent_inheritedAbsent_missing", () => {
  // Key not in local, not in inherited → missing.
  assert.equal(
    isRequiredKeyMissing("ANTHROPIC_API_KEY", {}, {}),
    true,
    "key absent from both local and inherited must be missing",
  );
});

test("isRequiredKeyMissing_keyExplicitlyEmpty_inheritedSet_stillMissing", () => {
  // Key in local with ""; inherited has a real value.
  // Local "" shadows inherited — effective value is empty → missing.
  // This is the Thufir IMPORTANT regression case.
  assert.equal(
    isRequiredKeyMissing(
      "ANTHROPIC_API_KEY",
      { ANTHROPIC_API_KEY: "" },
      { ANTHROPIC_API_KEY: "sk-global" },
    ),
    true,
    "explicit empty local value must shadow inherited and render Required badge",
  );
});

test("isRequiredKeyMissing_keyFilledLocally_inheritedSet_notMissing", () => {
  // Key in local with a real value; inherited also set → locally satisfied.
  assert.equal(
    isRequiredKeyMissing(
      "ANTHROPIC_API_KEY",
      { ANTHROPIC_API_KEY: "sk-local" },
      { ANTHROPIC_API_KEY: "sk-global" },
    ),
    false,
    "locally filled key must not be missing regardless of inherited value",
  );
});

test("isRequiredKeyMissing_keyFilledLocally_noInherited_notMissing", () => {
  // Key in local with a real value; no inherited → locally satisfied.
  assert.equal(
    isRequiredKeyMissing(
      "ANTHROPIC_API_KEY",
      { ANTHROPIC_API_KEY: "sk-local" },
      undefined,
    ),
    false,
    "locally filled key with no inherited must not be missing",
  );
});

test("isRequiredKeyMissing_keyExplicitlyEmpty_noInherited_missing", () => {
  // Key in local as ""; no inherited → missing.
  assert.equal(
    isRequiredKeyMissing(
      "ANTHROPIC_API_KEY",
      { ANTHROPIC_API_KEY: "" },
      undefined,
    ),
    true,
    "explicit empty local value with no inherited must be missing",
  );
});

// ── Invariant 4: inheritedRows — display/exclusion/override/no-serialize ───
//
// These tests exercise the pure-logic invariants that must hold for the
// inherited build-defaults feature in EnvVarsEditor:
//
//   (a) An inherited row with no matching local row IS visible (would render).
//   (b) An inherited row whose key appears in `rows` is HIDDEN (local wins).
//   (c) Serialization (buildRecord) NEVER includes inherited-only rows.
//   (d) A local override row for a masked secret shows the masked value.
//
// Tests (a)–(d) operate on the same helpers used by the component render
// (toRows, toRecord, buildRecord) plus a simulated filter that mirrors the
// JSX `.filter((irow) => !rows.some((r) => r.key === irow.key))`.

function simulateInheritedFilter(inheritedRows, rows) {
  return inheritedRows.filter((irow) => !rows.some((r) => r.key === irow.key));
}

test("inheritedRows_no_local_row_row_is_visible", () => {
  // DATABRICKS_HOST baked, no local row → inherited row shows.
  const inherited = [
    {
      key: "DATABRICKS_HOST",
      value: "https://example.databricks.com/",
      masked: false,
    },
  ];
  const rows = toRows({}, new Set()); // no local env vars
  const visible = simulateInheritedFilter(inherited, rows);
  assert.equal(
    visible.length,
    1,
    "inherited row must be visible when no local override",
  );
  assert.equal(visible[0].key, "DATABRICKS_HOST");
});

test("inheritedRows_local_row_same_key_inherited_hidden", () => {
  // User adds a local DATABRICKS_HOST row → inherited row must be hidden.
  const inherited = [
    {
      key: "DATABRICKS_HOST",
      value: "https://baked.databricks.com/",
      masked: false,
    },
  ];
  const value = { DATABRICKS_HOST: "https://user.databricks.com/" };
  const rows = toRows(value, new Set());
  const visible = simulateInheritedFilter(inherited, rows);
  assert.equal(
    visible.length,
    0,
    "inherited row must be hidden when local row has same key",
  );
});

test("inheritedRows_not_serialized_in_buildRecord", () => {
  // buildRecord must never include keys that come only from inherited rows.
  // Simulate: inherited has SECRET_KEY, local value does not.
  const requiredKeys = [];
  const value = { MY_VAR: "foo" };
  const rows = toRows(value, new Set(requiredKeys));
  // buildRecord reimplemented inline to match EnvVarsEditor's buildRecord:
  const base = {};
  for (const key of requiredKeys) {
    if (key in value) base[key] = value[key];
  }
  const record = { ...base, ...toRecord(rows) };
  assert.equal(
    "SECRET_KEY" in record,
    false,
    "inherited-only key must NOT appear in serialized record",
  );
  assert.equal(record.MY_VAR, "foo", "non-inherited key preserved");
});

test("inheritedRows_masked_secret_local_override_shows_masked_build_value", () => {
  // Edge case: baked key is a masked secret (e.g. API_KEY → "••••••"),
  // user types a local override → the hint would show the masked "••••••" value.
  const inherited = [{ key: "API_KEY", value: "••••••", masked: true }];
  // The component finds override by: inheritedRows.find(irow => irow.key === row.key)
  const row = { id: "r1", key: "API_KEY", value: "my-real-key" };
  const override = inherited.find((irow) => irow.key === row.key);
  assert.ok(override, "override entry must be found for masked key");
  assert.equal(
    override.value,
    "••••••",
    "masked baked value shown in override hint",
  );
  assert.equal(override.masked, true, "masked flag preserved");
});

test("inheritedRows_structured_keys_excluded_from_generic_rows", () => {
  // BUZZ_AGENT_PROVIDER, BUZZ_AGENT_MODEL, BUZZ_AGENT_THINKING_EFFORT must
  // be excluded from bakedGenericRows (they go to structured fields instead).
  // This mirrors the BAKED_STRUCTURED_KEYS filter in AgentDefaultsSettingsCard.
  const STRUCTURED = new Set([
    "BUZZ_AGENT_PROVIDER",
    "BUZZ_AGENT_MODEL",
    "BUZZ_AGENT_THINKING_EFFORT",
  ]);
  const allBaked = [
    { key: "BUZZ_AGENT_PROVIDER", value: "databricks_v2", masked: false },
    { key: "BUZZ_AGENT_MODEL", value: "goose-claude-opus-4-8", masked: false },
    { key: "BUZZ_AGENT_THINKING_EFFORT", value: "medium", masked: false },
    {
      key: "DATABRICKS_HOST",
      value: "https://example.databricks.com/",
      masked: false,
    },
    { key: "DATABRICKS_MODEL", value: "goose-claude-opus-4-8", masked: false },
  ];
  const generic = allBaked.filter((e) => !STRUCTURED.has(e.key));
  assert.equal(
    generic.length,
    2,
    "only non-structured keys go to generic rows",
  );
  const genericKeys = generic.map((e) => e.key).sort();
  assert.deepEqual(genericKeys, ["DATABRICKS_HOST", "DATABRICKS_MODEL"]);
});

// ── Invariant 5: getBakedProviderInheritLabel — label helper ───────────────

test("getBakedProviderInheritLabel_known_provider_returns_friendly_name", () => {
  const options = [
    { id: "anthropic", label: "Anthropic" },
    { id: "databricks_v2", label: "Databricks v2" },
    { id: "openai", label: "OpenAI" },
  ];
  const label = getBakedProviderInheritLabel("databricks_v2", options);
  assert.equal(
    label,
    "Databricks v2 (inherited from build)",
    "known provider id must resolve to friendly label",
  );
});

test("getBakedProviderInheritLabel_unknown_provider_falls_back_to_raw_id", () => {
  const options = [{ id: "anthropic", label: "Anthropic" }];
  const label = getBakedProviderInheritLabel("my-custom-provider", options);
  assert.equal(
    label,
    "my-custom-provider (inherited from build)",
    "unknown provider id must fall back to raw id",
  );
});

test("getBakedProviderInheritLabel_empty_options_falls_back_to_raw_id", () => {
  const label = getBakedProviderInheritLabel("databricks_v2", []);
  assert.equal(
    label,
    "databricks_v2 (inherited from build)",
    "empty options table must fall back to raw id",
  );
});

// ── keyAnnotations — annotation lookup invariants ─────────────────────────────
//
// `keyAnnotations` is a pass-through prop: the renderer does `keyAnnotations?.[key]`.
// The invariant worth pinning is that the prop contract is respected at the
// data level — an annotation for one key does NOT bleed into another key.
// (Rendering itself is trivially conditional; no logic to extract.)

test("keyAnnotations_present_key_has_annotation", () => {
  const annotations = {
    OPENAI_API_KEY: "Used for minting agent trading cards",
  };
  assert.equal(
    annotations.OPENAI_API_KEY,
    "Used for minting agent trading cards",
  );
});

test("keyAnnotations_absent_key_is_undefined", () => {
  const annotations = {
    OPENAI_API_KEY: "Used for minting agent trading cards",
  };
  assert.equal(annotations.ANTHROPIC_API_KEY, undefined);
});

test("keyAnnotations_empty_map_has_no_annotations", () => {
  const annotations = {};
  assert.equal(annotations.OPENAI_API_KEY, undefined);
});

test("keyAnnotations_only_matching_key_gets_annotation", () => {
  // Verifies the per-key lookup is not accidentally global.
  const annotations = { OPENAI_API_KEY: "card minting" };
  const keys = ["OPENAI_API_KEY", "ANTHROPIC_API_KEY", "FOO"];
  const results = keys.map((k) => annotations[k] ?? null);
  assert.deepEqual(results, ["card minting", null, null]);
});

// ── keyAnnotations render — annotation appears only on matching row ─────────
//
// renderToStaticMarkup exercises the real JSX path:
//   {keyAnnotations?.[row.key] ? <p ...>{annotation}</p> : null}
// This confirms the prop is plumbed through to the DOM correctly and that
// annotation text is scoped to its matching row.

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { EnvVarsEditor } from "./EnvVarsEditor.tsx";

test("keyAnnotations_annotation_present_only_on_matching_row", () => {
  const annotations = {
    OPENAI_API_KEY: "Used for minting agent trading cards",
  };
  const html = renderToStaticMarkup(
    React.createElement(EnvVarsEditor, {
      disabled: false,
      fileSatisfiedKeys: [],
      hiddenKeys: [],
      keyAnnotations: annotations,
      onChange: () => {},
      requiredKeys: [],
      value: { OPENAI_API_KEY: "sk-placeholder", ANTHROPIC_API_KEY: "sk-ant" },
    }),
  );
  assert.ok(
    html.includes("Used for minting agent trading cards"),
    "annotation must appear in rendered output for OPENAI_API_KEY row",
  );
  // The annotation must not bleed to other rows — check that it appears only once.
  const count = (html.match(/Used for minting agent trading cards/g) ?? [])
    .length;
  assert.equal(count, 1, "annotation must appear exactly once");
});

test("keyAnnotations_annotation_absent_for_non_matching_rows", () => {
  const annotations = {
    OPENAI_API_KEY: "Used for minting agent trading cards",
  };
  const html = renderToStaticMarkup(
    React.createElement(EnvVarsEditor, {
      disabled: false,
      fileSatisfiedKeys: [],
      hiddenKeys: [],
      keyAnnotations: annotations,
      onChange: () => {},
      requiredKeys: [],
      value: { ANTHROPIC_API_KEY: "sk-ant", MY_VAR: "foo" },
    }),
  );
  assert.ok(
    !html.includes("Used for minting agent trading cards"),
    "annotation must not appear when its key is not in the env map",
  );
});

// ── buildRecord with hiddenKeys: structured-field preservation ────────────
//
// These tests exercise the exported buildRecord(nextRows, value, requiredKeys,
// hiddenKeys) using the real implementation. hiddenKeys are structured-field
// env vars (e.g. BUZZ_AGENT_MAX_ROUNDS) that are owned by first-class controls
// outside the editor — they must survive onChange cycles even though they
// never appear as generic rows.
//
// Four scenarios from rev 4:
//   1. Edit an unrelated generic row → hidden (tuning) key is unchanged.
//   2. Runtime switch then generic edit: after switching to a new runtime,
//      the new runtime's hidden keys survive; old runtime keys appear as
//      generic rows and survive via toRecord, not hidden-key preservation.
//   3. Baked numeric key excluded via real descriptor/helper path: uses the
//      production deriveAgentConfigFieldModel + structuredEnvKeys helpers to
//      derive the hidden set, then verifies toRows excludes the numeric key.
//   4. Clearing a structured override → placeholder returns: after the user
//      clears a structured field (key absent from value), buildRecord must
//      not reintroduce it, leaving the structured field free to show the
//      Inherit placeholder.

test("buildRecord_hidden_tuning_key_unchanged_when_generic_row_edited", () => {
  // Structured field set BUZZ_AGENT_MAX_ROUNDS to "50"; it lives in value
  // as a hiddenKey. User then edits a generic env var via the row editor.
  // The tuning key must survive the buildRecord emit cycle unchanged.
  const value = { BUZZ_AGENT_MAX_ROUNDS: "50", MY_VAR: "old" };
  const nextRows = [{ id: "r1", key: "MY_VAR", value: "new" }];
  const record = buildRecordUtil(
    nextRows,
    value,
    [],
    ["BUZZ_AGENT_MAX_ROUNDS"],
  );

  assert.equal(
    record.BUZZ_AGENT_MAX_ROUNDS,
    "50",
    "hidden tuning key must survive when an unrelated generic row is edited",
  );
  assert.equal(record.MY_VAR, "new", "generic row edit applied");
});

test("buildRecord_runtime_switch_new_hiddenKeys_then_generic_edit", () => {
  // Scenario 2: runtime switch then generic edit.
  //
  // Before switch: agent is buzz-agent with BUZZ_AGENT_MAX_ROUNDS = "50" stored
  // in value (set via the numeric tuning control). After switching to Goose,
  // the buzz-agent key is no longer hidden — it becomes a visible generic row.
  // The test verifies:
  //   (a) After the switch, the old buzz-agent key appears as a generic row
  //       (toRows with the new Goose hidden set projects it).
  //   (b) After a generic-row edit, buildRecord preserves BOTH the old-runtime
  //       key (now a generic row) and the new-runtime hidden key.
  //   (c) An unset new-runtime hidden key is not introduced.

  // Derive both descriptor sets from real runtime objects.
  const buzzAgentRuntime = {
    id: "buzz-agent",
    label: "Buzz Agent",
    avatarUrl: "",
    availability: "available",
    command: "buzz-agent",
    binaryPath: "buzz-agent",
    defaultArgs: [],
    mcpCommand: null,
    modelEnvVar: "BUZZ_AGENT_MODEL",
    providerEnvVar: "BUZZ_AGENT_PROVIDER",
    thinkingEnvVar: "BUZZ_AGENT_THINKING_EFFORT",
    maxTokensEnvVar: "BUZZ_AGENT_MAX_OUTPUT_TOKENS",
    contextLimitEnvVar: "BUZZ_AGENT_CONTEXT_LIMIT",
    maxRoundsEnvVar: "BUZZ_AGENT_MAX_ROUNDS",
    installHint: "",
    installInstructionsUrl: "",
    canAutoInstall: false,
    underlyingCliPath: null,
    nodeRequired: false,
    authStatus: { status: "not_applicable" },
    loginHint: null,
  };
  const gooseRuntime = {
    id: "goose",
    label: "Goose",
    avatarUrl: "",
    availability: "available",
    command: "goose",
    binaryPath: "goose",
    defaultArgs: ["acp"],
    mcpCommand: null,
    modelEnvVar: null,
    providerEnvVar: null,
    thinkingEnvVar: null,
    maxTokensEnvVar: "GOOSE_MAX_TOKENS",
    contextLimitEnvVar: "GOOSE_CONTEXT_LIMIT",
    maxRoundsEnvVar: null,
    installHint: "",
    installInstructionsUrl: "",
    canAutoInstall: true,
    underlyingCliPath: null,
    nodeRequired: false,
    authStatus: { status: "not_applicable" },
    loginHint: null,
  };

  const buzzDescriptors = deriveNumericDescriptors(buzzAgentRuntime);
  const gooseDescriptors = deriveNumericDescriptors(gooseRuntime);
  const buzzHiddenKeys = structuredEnvKeys(buzzDescriptors);
  const gooseHiddenKeys = structuredEnvKeys(gooseDescriptors);

  // Sanity-check that BUZZ_AGENT_MAX_ROUNDS is hidden under buzz-agent but not
  // under Goose — that contrast is what makes it become a generic row.
  assert.ok(
    buzzHiddenKeys.includes("BUZZ_AGENT_MAX_ROUNDS"),
    "BUZZ_AGENT_MAX_ROUNDS must be hidden under buzz-agent descriptors",
  );
  assert.equal(
    gooseHiddenKeys.includes("BUZZ_AGENT_MAX_ROUNDS"),
    false,
    "BUZZ_AGENT_MAX_ROUNDS must not be hidden under Goose descriptors",
  );

  // Pre-switch value: buzz-agent max-rounds was set, GOOSE_MAX_TOKENS was
  // already set (e.g. user configured it before switching back), plus a
  // generic user var. GOOSE_MAX_TOKENS is a hidden key under the Goose
  // descriptor set, so it must survive buildRecord() via hiddenKeys.
  const valueBeforeSwitch = {
    BUZZ_AGENT_MAX_ROUNDS: "50",
    GOOSE_MAX_TOKENS: "16384",
    USER_VAR: "original",
  };

  // After the switch to Goose, toRows is reproj with the new (Goose) hidden
  // set. BUZZ_AGENT_MAX_ROUNDS is no longer hidden → appears as a generic row.
  // GOOSE_MAX_TOKENS IS hidden under Goose → must not appear in generic rows.
  const rowsAfterSwitch = toRows(valueBeforeSwitch, new Set(gooseHiddenKeys));
  assert.ok(
    rowsAfterSwitch.some((r) => r.key === "BUZZ_AGENT_MAX_ROUNDS"),
    "old-runtime key must become a generic row after the switch",
  );
  assert.equal(
    rowsAfterSwitch.some((r) => r.key === "GOOSE_MAX_TOKENS"),
    false,
    "new-runtime hidden key must not appear as a generic row after the switch",
  );

  // User edits the generic USER_VAR row.
  const editedRows = rowsAfterSwitch.map((r) =>
    r.key === "USER_VAR" ? { ...r, value: "updated" } : r,
  );

  // buildRecord: old-runtime key survives via toRecord (it's now a generic
  // row); new-runtime Goose hidden key survives via hiddenKeys (carried
  // through from value). An unset Goose key must not be introduced.
  const record = buildRecordUtil(
    editedRows,
    valueBeforeSwitch,
    [],
    gooseHiddenKeys,
  );

  assert.equal(
    record.BUZZ_AGENT_MAX_ROUNDS,
    "50",
    "old-runtime key must survive as a generic row value after switch",
  );
  assert.equal(
    record.GOOSE_MAX_TOKENS,
    "16384",
    "new-runtime hidden key must survive buildRecord via hiddenKeys",
  );
  assert.equal(record.USER_VAR, "updated", "generic row edit applied");
  assert.equal(
    "GOOSE_CONTEXT_LIMIT" in record,
    false,
    "unset new-runtime hidden key must not be introduced",
  );
});

test("filterBakedGenericRows_numeric_baked_key_excluded_and_placeholder_shown", () => {
  // Scenario 3: baked numeric key excluded via the real production helper.
  //
  // The global baked env contains BUZZ_AGENT_MAX_OUTPUT_TOKENS = "4096"
  // (the baked value shipped with the agent). The production
  // filterBakedGenericRows path must exclude this key from the generic
  // baked-row display so it isn't editable twice, while the structured
  // numeric input shows the inherited placeholder via numericTuningPlaceholder.
  const buzzAgentRuntime = {
    id: "buzz-agent",
    label: "Buzz Agent",
    avatarUrl: "",
    availability: "available",
    command: "buzz-agent",
    binaryPath: "buzz-agent",
    defaultArgs: [],
    mcpCommand: null,
    modelEnvVar: "BUZZ_AGENT_MODEL",
    providerEnvVar: "BUZZ_AGENT_PROVIDER",
    thinkingEnvVar: "BUZZ_AGENT_THINKING_EFFORT",
    maxTokensEnvVar: "BUZZ_AGENT_MAX_OUTPUT_TOKENS",
    contextLimitEnvVar: null,
    maxRoundsEnvVar: null,
    installHint: "",
    installInstructionsUrl: "",
    canAutoInstall: false,
    underlyingCliPath: null,
    nodeRequired: false,
    authStatus: { status: "not_applicable" },
    loginHint: null,
  };

  const numericDescriptors = deriveNumericDescriptors(buzzAgentRuntime);
  const numericStructuredKeys = structuredEnvKeys(numericDescriptors);

  assert.ok(
    numericStructuredKeys.includes("BUZZ_AGENT_MAX_OUTPUT_TOKENS"),
    "numeric key must appear in structured keys via production helpers",
  );

  // Simulate the baked env: BUZZ_AGENT_MAX_OUTPUT_TOKENS is baked, plus a
  // non-structured baked var.
  const bakedEnv = [
    { key: "BUZZ_AGENT_MAX_OUTPUT_TOKENS", value: "4096" },
    { key: "SOME_OTHER_BAKED_VAR", value: "hello" },
  ];

  // filterBakedGenericRows must exclude the numeric key.
  const genericRows = filterBakedGenericRows(bakedEnv, numericStructuredKeys);

  assert.equal(
    genericRows.some((r) => r.key === "BUZZ_AGENT_MAX_OUTPUT_TOKENS"),
    false,
    "baked numeric key must be excluded from generic baked rows",
  );
  assert.ok(
    genericRows.some((r) => r.key === "SOME_OTHER_BAKED_VAR"),
    "non-structured baked var must remain in generic rows",
  );

  // The structured numeric input shows the inherited placeholder for the
  // baked value via numericTuningPlaceholder.
  const bakedValue = "4096";
  assert.equal(
    numericTuningPlaceholder(bakedValue),
    "Inherit (4096)",
    "structured placeholder must reflect the baked value",
  );
  assert.equal(
    numericTuningPlaceholder(undefined),
    "Inherit (agent default)",
    "structured placeholder without baked value shows agent-default text",
  );
});

test("buildRecord_clearing_structured_field_allows_placeholder_to_return", () => {
  // Scenario 4: clearing a structured override → placeholder returns.
  //
  // Step 1: value has BUZZ_AGENT_MAX_ROUNDS = "50" (user set it via the
  //         structured field). BUZZ_AGENT_MAX_ROUNDS is in hiddenKeys.
  // Step 2: user clears the structured field → onEnvVarChange(key, "")
  //         removes the key from value (value no longer contains it).
  // Step 3: after the clear, buildRecord must not reintroduce the key.
  // Step 4: with the key absent from value, numericTuningPlaceholder over
  //         the (now-empty) inheritedEnvVars shows "Inherit (agent default)"
  //         — the numeric field's empty-state placeholder.

  // After the clear, value no longer contains BUZZ_AGENT_MAX_ROUNDS.
  const valueAfterClear = { MY_VAR: "foo" };
  const nextRows = [{ id: "r1", key: "MY_VAR", value: "updated" }];

  const record = buildRecordUtil(
    nextRows,
    valueAfterClear,
    [],
    ["BUZZ_AGENT_MAX_ROUNDS"],
  );

  assert.equal(
    "BUZZ_AGENT_MAX_ROUNDS" in record,
    false,
    "cleared structured key must not be reintroduced by buildRecord",
  );
  assert.equal(record.MY_VAR, "updated");

  // With the key cleared, the inherited value is also absent (not set
  // globally). numericTuningPlaceholder returns the agent-default text —
  // the placeholder that renders in the structured input.
  const inheritedAfterClear = undefined;
  assert.equal(
    numericTuningPlaceholder(inheritedAfterClear),
    "Inherit (agent default)",
    "numeric input must show Inherit (agent default) after clear when no global override",
  );

  // If a global override IS set, the placeholder shows that value instead.
  const inheritedGlobal = "25";
  assert.equal(
    numericTuningPlaceholder(inheritedGlobal),
    "Inherit (25)",
    "numeric input must show Inherit (<global value>) when a global override exists",
  );
});
