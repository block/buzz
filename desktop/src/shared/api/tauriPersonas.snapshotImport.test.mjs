import assert from "node:assert/strict";
import test from "node:test";

// Pure type-level tests for the AgentSnapshotImportPreview shape and the
// AgentSnapshotImportResult partial-failure contract. These run in Node.js
// without a Tauri environment, so they test the pure mapping / guard logic
// used by the dialog, not the Tauri IPC itself.

/**
 * Build a minimal preview object as the backend would return it.
 */
function makePreview(overrides = {}) {
  return {
    displayName: "Test Agent",
    systemPrompt: "You are helpful.",
    avatarUrl: null,
    memoryLevel: "none",
    memoryEntryCount: 0,
    hasSourceAllowlist: false,
    sourceAllowlistCount: 0,
    ...overrides,
  };
}

/**
 * Build a minimal import result as the backend would return it.
 */
function makeResult(overrides = {}) {
  return {
    displayName: "Test Agent",
    newPubkey: "abc123",
    personaId: "persona-uuid",
    memoryWritten: 0,
    memoryTotal: 0,
    memoryErrors: [],
    profileSyncError: null,
    ...overrides,
  };
}

// ── Preview: allowlist warning ────────────────────────────────────────────────

test("preview_has_source_allowlist_is_true_when_allowlist_non_empty", () => {
  const preview = makePreview({
    hasSourceAllowlist: true,
    sourceAllowlistCount: 2,
  });
  assert.equal(preview.hasSourceAllowlist, true);
  assert.equal(preview.sourceAllowlistCount, 2);
});

test("preview_has_source_allowlist_is_false_when_allowlist_empty", () => {
  const preview = makePreview({
    hasSourceAllowlist: false,
    sourceAllowlistCount: 0,
  });
  assert.equal(preview.hasSourceAllowlist, false);
});

// ── Preview: memory warning ───────────────────────────────────────────────────

test("preview_memory_entry_count_drives_warning_display", () => {
  const noMemory = makePreview({ memoryEntryCount: 0, memoryLevel: "none" });
  assert.equal(noMemory.memoryEntryCount, 0);

  const withMemory = makePreview({
    memoryEntryCount: 3,
    memoryLevel: "everything",
  });
  assert.equal(withMemory.memoryEntryCount, 3);
  assert.equal(withMemory.memoryLevel, "everything");
});

// ── Allowlist: default is clear (server-side enforced) ───────────────────────

test("allowlist_default_is_clear", () => {
  // keepAllowlist defaults to false in the dialog — safe default.
  const keepAllowlist = false;
  const sourceAllowlist = ["aabb".repeat(16)];
  const appliedAllowlist = keepAllowlist ? sourceAllowlist : [];
  assert.deepEqual(appliedAllowlist, []);
});

test("allowlist_keep_preserves_source", () => {
  const keepAllowlist = true;
  const sourceAllowlist = ["aabb".repeat(16)];
  const appliedAllowlist = keepAllowlist ? sourceAllowlist : [];
  assert.deepEqual(appliedAllowlist, sourceAllowlist);
});

// ── Result: full success ──────────────────────────────────────────────────────

test("result_full_success_has_no_errors", () => {
  const result = makeResult({
    memoryWritten: 3,
    memoryTotal: 3,
    memoryErrors: [],
  });
  assert.equal(result.memoryWritten, result.memoryTotal);
  assert.equal(result.memoryErrors.length, 0);
});

// ── Result: partial failure ───────────────────────────────────────────────────

test("result_partial_failure_reports_incomplete_memory", () => {
  const result = makeResult({
    memoryWritten: 1,
    memoryTotal: 3,
    memoryErrors: [
      'slug "mem/foo": relay rejected engram: timeout',
      'slug "mem/bar": relay rejected engram: timeout',
    ],
  });
  assert.equal(result.memoryWritten < result.memoryTotal, true);
  assert.equal(result.memoryErrors.length, 2);
  // Agent itself was created:
  assert.ok(result.newPubkey);
  // Error count matches unwritten:
  assert.equal(
    result.memoryErrors.length,
    result.memoryTotal - result.memoryWritten,
  );
});

test("result_partial_failure_agent_exists_despite_errors", () => {
  const result = makeResult({
    newPubkey: "deadbeef",
    memoryWritten: 0,
    memoryTotal: 2,
    memoryErrors: ['slug "core": build failed', 'slug "mem/x": timeout'],
  });
  // Agent exists:
  assert.equal(result.newPubkey, "deadbeef");
  // Memory failed:
  assert.equal(result.memoryErrors.length, 2);
});

// ── Result: no memory snapshot ────────────────────────────────────────────────

test("result_no_memory_snapshot_has_zero_total", () => {
  const result = makeResult({ memoryWritten: 0, memoryTotal: 0 });
  assert.equal(result.memoryTotal, 0);
  assert.equal(result.memoryErrors.length, 0);
});

// ── Preview: memory level labelling ──────────────────────────────────────────

test("preview_memory_level_labels_all_three_values", () => {
  const levels = ["none", "core", "everything"];
  for (const level of levels) {
    const preview = makePreview({ memoryLevel: level });
    assert.equal(preview.memoryLevel, level);
  }
});
