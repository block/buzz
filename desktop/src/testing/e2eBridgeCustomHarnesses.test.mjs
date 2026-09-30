/**
 * Unit tests for the e2eBridge custom harness handlers (C-10).
 *
 * Tests are imported directly from the extracted e2eBridgeCustomHarnesses.ts
 * module — no Tauri mock or browser environment needed — to prove:
 *
 *  1. save returns a catalog entry with source "custom" and correct fields
 *  2. definition_env is preserved through save (non-empty env)
 *  3. empty env produces absent definition_env (mirrors Rust BTreeMap serialization)
 *  4. same-ID edit replaces the existing entry in the store (no duplicates)
 *  5. rename (originalId ≠ id) removes the old key and inserts the new one
 *  6. delete removes the entry from the store
 *  7. delete is idempotent (not-found does not throw)
 *  8. discover integration: saved harness appears in the discover result set
 *     alongside the default catalog (verifies the Map is shared by reference)
 */
import assert from "node:assert/strict";
import { beforeEach, describe, it } from "node:test";

import {
  mockCustomHarnesses,
  mergeMockCustomHarnesses,
  resetMockCustomHarnesses,
  handleSaveCustomHarness,
  handleDeleteCustomHarness,
} from "./e2eBridgeCustomHarnesses.ts";

function makeArgs(overrides = {}) {
  return {
    definition: {
      id: overrides.id ?? "test-harness",
      label: overrides.label ?? "Test Harness",
      command: overrides.command ?? "test-bin",
      args: overrides.args ?? [],
      env: overrides.env ?? {},
      installInstructionsUrl: overrides.installInstructionsUrl ?? "",
      installHint: overrides.installHint ?? "",
    },
    originalId: overrides.originalId ?? null,
  };
}

// Reset the store before every test so tests are independent.
beforeEach(() => resetMockCustomHarnesses());

// ── save_custom_harness ───────────────────────────────────────────────────────

describe("handleSaveCustomHarness", () => {
  it("returns a catalog entry with source 'custom'", () => {
    const entry = handleSaveCustomHarness(
      makeArgs({ id: "my-rt", label: "My RT" }),
    );
    assert.equal(entry.id, "my-rt");
    assert.equal(entry.label, "My RT");
    assert.equal(entry.source, "custom");
  });

  it("stores the entry in mockCustomHarnesses", () => {
    handleSaveCustomHarness(makeArgs({ id: "stored" }));
    assert.ok(
      mockCustomHarnesses.has("stored"),
      "store must contain the saved id",
    );
  });

  it("preserves non-empty definition_env", () => {
    const env = { ANTHROPIC_API_KEY: "sk-test", MODEL: "claude-3" };
    const entry = handleSaveCustomHarness(makeArgs({ id: "env-rt", env }));
    assert.deepStrictEqual(entry.definition_env, env);
    assert.deepStrictEqual(
      mockCustomHarnesses.get("env-rt")?.definition_env,
      env,
    );
  });

  it("produces absent definition_env for empty env (mirrors Rust BTreeMap skip)", () => {
    const entry = handleSaveCustomHarness(makeArgs({ id: "no-env", env: {} }));
    assert.ok(
      entry.definition_env === undefined || entry.definition_env === null,
      "empty env must yield absent definition_env",
    );
  });

  it("same-ID edit replaces entry — no duplicates in the store", () => {
    handleSaveCustomHarness(makeArgs({ id: "dup", label: "V1" }));
    handleSaveCustomHarness(
      makeArgs({ id: "dup", label: "V2", originalId: "dup" }),
    );
    assert.equal(
      mockCustomHarnesses.size,
      1,
      "same-ID edit must not duplicate store entries",
    );
    assert.equal(mockCustomHarnesses.get("dup")?.label, "V2");
  });

  it("rename removes old key and inserts new key", () => {
    handleSaveCustomHarness(makeArgs({ id: "old-rt", label: "Old" }));
    handleSaveCustomHarness(
      makeArgs({ id: "new-rt", label: "New", originalId: "old-rt" }),
    );
    assert.ok(
      !mockCustomHarnesses.has("old-rt"),
      "old key must be removed on rename",
    );
    assert.ok(
      mockCustomHarnesses.has("new-rt"),
      "new key must be present after rename",
    );
    assert.equal(mockCustomHarnesses.get("new-rt")?.label, "New");
  });
});

// ── delete_custom_harness ────────────────────────────────────────────────────

describe("handleDeleteCustomHarness", () => {
  it("removes an existing entry from the store", () => {
    handleSaveCustomHarness(makeArgs({ id: "to-delete" }));
    assert.ok(mockCustomHarnesses.has("to-delete"));

    handleDeleteCustomHarness({ id: "to-delete" });
    assert.ok(
      !mockCustomHarnesses.has("to-delete"),
      "entry must be removed after delete",
    );
  });

  it("is idempotent — deleting non-existent id does not throw", () => {
    assert.doesNotThrow(
      () => handleDeleteCustomHarness({ id: "never-existed" }),
      "delete of non-existent id must not throw",
    );
  });

  it("throws when deleteCustomHarnessError knob is set", () => {
    handleSaveCustomHarness(makeArgs({ id: "keep-alive" }));
    const config = { mock: { deleteCustomHarnessError: "permission denied" } };
    assert.throws(
      () => handleDeleteCustomHarness({ id: "keep-alive" }, config),
      /permission denied/,
      "must throw the injected error message",
    );
    // Entry must remain in store — the error means delete did not complete.
    assert.ok(
      mockCustomHarnesses.has("keep-alive"),
      "entry must remain when delete throws",
    );
  });
});

// ── discover integration: store is shared by reference ───────────────────────

describe("mergeMockCustomHarnesses", () => {
  const seeded = (id, label = id) => ({ id, label, source: "custom" });

  it("appends a newly saved harness that is not in the seeded catalog", () => {
    handleSaveCustomHarness(makeArgs({ id: "added", label: "Added" }));
    const merged = mergeMockCustomHarnesses([seeded("preset-a")]);
    assert.deepEqual(
      merged.map((e) => e.id),
      ["preset-a", "added"],
    );
  });

  it("replaces a seeded entry in place on same-id save — no duplicate row", () => {
    handleSaveCustomHarness(makeArgs({ id: "seeded-one", label: "V2" }));
    const merged = mergeMockCustomHarnesses([
      seeded("preset-a"),
      seeded("seeded-one", "V1"),
    ]);
    assert.deepEqual(
      merged.map((e) => e.id),
      ["preset-a", "seeded-one"],
      "must not duplicate the id",
    );
    assert.equal(
      merged.find((e) => e.id === "seeded-one").label,
      "V2",
      "saved entry must win over the seed",
    );
  });

  it("drops a deleted seeded entry (tombstone, not just store removal)", () => {
    // The regression: a seeded row has no store entry to delete, so without a
    // tombstone the row survived the delete and the spec failed.
    handleDeleteCustomHarness({ id: "seeded-one" });
    const merged = mergeMockCustomHarnesses([
      seeded("preset-a"),
      seeded("seeded-one"),
    ]);
    assert.deepEqual(
      merged.map((e) => e.id),
      ["preset-a"],
    );
  });

  it("drops the vacated old id after a rename and surfaces the new one", () => {
    handleSaveCustomHarness(
      makeArgs({ id: "new-id", label: "New", originalId: "old-id" }),
    );
    const merged = mergeMockCustomHarnesses([seeded("old-id", "Old")]);
    assert.deepEqual(
      merged.map((e) => e.id),
      ["new-id"],
    );
  });

  it("re-saving a deleted id resurrects it", () => {
    handleDeleteCustomHarness({ id: "seeded-one" });
    handleSaveCustomHarness(makeArgs({ id: "seeded-one", label: "Back" }));
    const merged = mergeMockCustomHarnesses([seeded("seeded-one", "Original")]);
    assert.deepEqual(
      merged.map((e) => e.id),
      ["seeded-one"],
    );
    assert.equal(merged[0].label, "Back");
  });

  it("leaves a seeded catalog untouched when nothing has been mutated", () => {
    const base = [seeded("preset-a"), seeded("preset-b")];
    assert.deepEqual(
      mergeMockCustomHarnesses(base).map((e) => e.id),
      ["preset-a", "preset-b"],
    );
  });
});

describe("mockCustomHarnesses Map reference", () => {
  it("handler writes are immediately visible to callers that read the exported Map", () => {
    // handleDiscoverAcpRuntimes merges this store into the catalog it returns.
    // The exported Map is the same object by reference, so writes via the
    // handler are visible to any reader of the Map.
    assert.equal(
      mockCustomHarnesses.size,
      0,
      "store must be empty after reset",
    );

    handleSaveCustomHarness(makeArgs({ id: "visible", label: "Visible" }));
    assert.equal(mockCustomHarnesses.size, 1);

    const [entry] = Array.from(mockCustomHarnesses.values());
    assert.equal(entry.id, "visible");
    assert.equal(entry.source, "custom");
  });
});
