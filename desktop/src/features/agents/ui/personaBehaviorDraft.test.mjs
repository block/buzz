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
    parallelism: 3,
  };
  const draft = draftFromBehavior(behavior);
  assert.deepEqual(draft, {
    respondTo: "allowlist",
    respondToAllowlist: [HEX],
    parallelism: "3",
  });
  draft.respondToAllowlist.push("mutated");
  assert.deepEqual(behavior.respondToAllowlist, [HEX], "list must be copied");
  assert.deepEqual(draftFromBehavior(undefined), emptyPersonaBehaviorDraft);
});

test("edit full-clear submits an explicit empty group, not nothing", () => {
  // Pinky's 48f260a11 finding: a mode-less quad (toolsets/parallelism only)
  // cleared to completely empty must still submit, or the stored quad
  // silently resurrects on reopen.
  const seed = {
    ...emptyPersonaBehaviorDraft,
    parallelism: "4",
  };
  const group = behaviorForSubmit(emptyPersonaBehaviorDraft, seed, true);
  assert.deepEqual(group, {}, "full clear must submit a replace-with-empty");
  // Partial clear keeps working: one field left set submits that field.
  const partial = behaviorForSubmit({ ...seed, parallelism: "8" }, seed, true);
  assert.equal(partial.parallelism, 8);
  // Hash-quiet survives the fix: a no-op edit of an ALREADY-quad-less
  // definition still submits nothing — `{}` here would republish and flip
  // content hashes for exactly the definitions the hash-quiet row protects.
  const noop = behaviorForSubmit(
    emptyPersonaBehaviorDraft,
    emptyPersonaBehaviorDraft,
    true,
  );
  assert.equal(noop, undefined, "empty-vs-empty must stay silent");
});
