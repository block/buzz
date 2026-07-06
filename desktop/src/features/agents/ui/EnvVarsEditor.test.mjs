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
 *
 * These are pure-logic tests — no React renderer needed. The transition tests
 * (Invariant 3) exercise the real exported `skipKeysEqual` guard that controls
 * whether the effect calls `setRows(toRows(value, skipKeys))`.
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import { toRows, toRecord, skipKeysEqual } from "./EnvVarsEditor.tsx";

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
