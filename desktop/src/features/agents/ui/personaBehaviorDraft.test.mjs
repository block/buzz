import assert from "node:assert/strict";
import test from "node:test";

import {
  behaviorForSubmit,
  draftFromBehavior,
  emptyPersonaBehaviorDraft,
  personaBehaviorDraftValid,
} from "./personaBehaviorDraft.ts";

const HEX = "a".repeat(64);

function allowlistDraft(overrides = {}) {
  return {
    ...emptyPersonaBehaviorDraft,
    respondTo: "allowlist",
    respondToAllowlist: [HEX],
    ...overrides,
  };
}

// ── Crash-loop guard (Q2: create AND edit both route through this) ──────────

test("allowlist mode with an empty list is invalid", () => {
  assert.equal(
    personaBehaviorDraftValid(allowlistDraft({ respondToAllowlist: [] })),
    false,
  );
  assert.equal(personaBehaviorDraftValid(allowlistDraft()), true);
  assert.equal(personaBehaviorDraftValid(emptyPersonaBehaviorDraft), true);
});

// ── Q5 parse fidelity (legacy dialog parity) ─────────────────────────────────

test("parallelism submits only when parseInt > 0", () => {
  for (const bad of ["", "0", "-2", "abc"]) {
    const group = behaviorForSubmit(
      { ...emptyPersonaBehaviorDraft, parallelism: bad },
      emptyPersonaBehaviorDraft,
      false,
    );
    assert.equal(group?.parallelism, undefined, `"${bad}" must not submit`);
  }
  const group = behaviorForSubmit(
    { ...emptyPersonaBehaviorDraft, parallelism: "4" },
    emptyPersonaBehaviorDraft,
    false,
  );
  assert.equal(group.parallelism, 4);
});

test("mcpToolsets trims to undefined when empty", () => {
  const blank = behaviorForSubmit(
    { ...emptyPersonaBehaviorDraft, mcpToolsets: "   " },
    emptyPersonaBehaviorDraft,
    false,
  );
  assert.equal(blank, undefined, "whitespace-only toolsets is an empty quad");
  const set = behaviorForSubmit(
    { ...emptyPersonaBehaviorDraft, mcpToolsets: " developer " },
    emptyPersonaBehaviorDraft,
    false,
  );
  assert.equal(set.mcpToolsets, "developer");
});

// ── Unit semantics: mode and list travel together ────────────────────────────

test("flipping allowlist back to owner-only drops the list from the submit", () => {
  const group = behaviorForSubmit(
    allowlistDraft({ respondTo: "owner-only" }),
    emptyPersonaBehaviorDraft,
    false,
  );
  assert.equal(group.respondTo, "owner-only");
  assert.equal(
    group.respondToAllowlist,
    undefined,
    "stale pubkeys must not ride a non-allowlist mode",
  );
});

// ── Absent-vs-present: unrelated edits must not touch the quad ───────────────

test("create with an untouched empty quad submits nothing", () => {
  assert.equal(
    behaviorForSubmit(
      emptyPersonaBehaviorDraft,
      emptyPersonaBehaviorDraft,
      false,
    ),
    undefined,
  );
});

test("edit with an untouched quad submits nothing (hash-quiet unrelated edits)", () => {
  const seed = allowlistDraft();
  assert.equal(behaviorForSubmit(allowlistDraft(), seed, true), undefined);
});

test("edit with a changed quad submits the full group", () => {
  const seed = allowlistDraft();
  const group = behaviorForSubmit(
    allowlistDraft({ respondToAllowlist: [HEX, "b".repeat(64)] }),
    seed,
    true,
  );
  assert.deepEqual(group, {
    respondTo: "allowlist",
    respondToAllowlist: [HEX, "b".repeat(64)],
    mcpToolsets: undefined,
    parallelism: undefined,
  });
});

test("duplicate (create with inherited seed) submits the inherited quad", () => {
  const seed = allowlistDraft();
  const group = behaviorForSubmit(allowlistDraft(), seed, false);
  assert.equal(group.respondTo, "allowlist");
  assert.deepEqual(group.respondToAllowlist, [HEX]);
});

// ── Draft seeding round-trip ─────────────────────────────────────────────────

test("draftFromBehavior round-trips a full quad and copies the list", () => {
  const behavior = {
    respondTo: "allowlist",
    respondToAllowlist: [HEX],
    mcpToolsets: "developer",
    parallelism: 3,
  };
  const draft = draftFromBehavior(behavior);
  assert.deepEqual(draft, {
    respondTo: "allowlist",
    respondToAllowlist: [HEX],
    mcpToolsets: "developer",
    parallelism: "3",
  });
  draft.respondToAllowlist.push("mutated");
  assert.deepEqual(behavior.respondToAllowlist, [HEX], "list must be copied");
  assert.deepEqual(draftFromBehavior(undefined), emptyPersonaBehaviorDraft);
});
