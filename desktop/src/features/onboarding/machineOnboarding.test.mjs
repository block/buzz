import assert from "node:assert/strict";
import test from "node:test";

import {
  migrateMachineOnboardingCompletion,
  readMachineOnboardingCompletion,
} from "./machineOnboarding.ts";

// machineOnboarding.ts reads/writes window.localStorage directly, so we inject
// a minimal in-memory storage into globalThis.window before each test and
// restore it afterward.

function createMemoryStorage(initial = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: (key) => values.delete(key),
    clear: () => values.clear(),
    key: (index) => Array.from(values.keys())[index] ?? null,
    get length() {
      return values.size;
    },
  };
}

function withFakeWindow(initial, fn) {
  const prev = globalThis.window;
  const storage = createMemoryStorage(initial);
  globalThis.window = {
    localStorage: storage,
    // location.href without a machineOnboarding=1 param → forceMachineOnboarding() returns false
    location: { href: "http://localhost/" },
  };
  try {
    return fn(storage);
  } finally {
    globalThis.window = prev;
  }
}

const PUBKEY_A =
  "aaaaaa1111112222223333334444445555556666667777778888889999990000aa";
const PUBKEY_B =
  "bbbbbb1111112222223333334444445555556666667777778888889999990000bb";
const LEGACY_KEY = `buzz-onboarding-complete.v1:${PUBKEY_A}`;
const V2_KEY = `buzz-machine-onboarding-complete.v2:${PUBKEY_A}`;

// ── Fix A regression case ────────────────────────────────────────────────────

test("migrate_mismatched_community_pubkey_does_not_vouch_for_current_key", () => {
  // Community pubkey is PUBKEY_B (stale from a previous identity); current
  // pubkey is PUBKEY_A (freshly generated after a dev reset). The mismatch
  // must NOT grant completion.
  withFakeWindow({}, (storage) => {
    const result = migrateMachineOnboardingCompletion(
      PUBKEY_A,
      PUBKEY_B, // activeCommunityPubkey — a different identity's community
      false,
    );
    assert.equal(result, false, "mismatched pubkey must not vouch");
    assert.equal(
      storage.getItem(V2_KEY),
      null,
      "completion must not be written to storage",
    );
    assert.equal(
      readMachineOnboardingCompletion(PUBKEY_A),
      false,
      "readMachineOnboardingCompletion must return false",
    );
  });
});

// ── Matching pubkey vouches ──────────────────────────────────────────────────

test("migrate_matching_community_pubkey_vouches_for_current_key", () => {
  // Community pubkey matches current pubkey → legitimate veteran, grant.
  withFakeWindow({}, (storage) => {
    const result = migrateMachineOnboardingCompletion(
      PUBKEY_A,
      PUBKEY_A, // same pubkey — community belongs to this identity
      false,
    );
    assert.equal(result, true, "matching pubkey must vouch");
    assert.equal(storage.getItem(V2_KEY), "true");
    assert.equal(readMachineOnboardingCompletion(PUBKEY_A), true);
  });
});

// ── Absent community pubkey — regression case (Thufir pass 1) ───────────────

test("migrate_absent_community_pubkey_does_not_vouch_for_fresh_identity", () => {
  // Stale current-format community with no pubkey stamp (created before the
  // stamp was added, or from an older build) + freshly generated identity
  // after a dev reset. The absent pubkey must NOT vouch — this was the
  // reachable producer of the half-onboarded state.
  withFakeWindow({}, (storage) => {
    const result = migrateMachineOnboardingCompletion(
      PUBKEY_A,
      null, // absent pubkey — could be legacy OR an unstamped modern entry
      false,
    );
    assert.equal(
      result,
      false,
      "absent pubkey must not vouch for a fresh identity",
    );
    assert.equal(
      storage.getItem(V2_KEY),
      null,
      "completion must not be written to storage",
    );
  });
});

// ── No community configured ──────────────────────────────────────────────────

test("migrate_no_community_does_not_vouch_for_current_key", () => {
  // `undefined` = no active community at all.
  withFakeWindow({}, (storage) => {
    const result = migrateMachineOnboardingCompletion(
      PUBKEY_A,
      undefined, // no community
      false,
    );
    assert.equal(result, false, "no community must not vouch");
    assert.equal(storage.getItem(V2_KEY), null);
  });
});

// ── Legacy onboarding completion still grants (pubkey-scoped) ───────────────

test("migrate_legacy_completion_key_still_grants_migration", () => {
  withFakeWindow({ [LEGACY_KEY]: "true" }, (storage) => {
    const result = migrateMachineOnboardingCompletion(
      PUBKEY_A,
      undefined, // no community — but legacy key is present
      false,
    );
    assert.equal(result, true, "legacy completion key must grant migration");
    assert.equal(storage.getItem(V2_KEY), "true");
  });
});

// ── Shared identity grants regardless of community ───────────────────────────

test("migrate_shared_identity_grants_regardless_of_community", () => {
  withFakeWindow({}, () => {
    const result = migrateMachineOnboardingCompletion(
      PUBKEY_A,
      undefined, // no community
      true, // isSharedIdentity
    );
    assert.equal(result, true, "shared identity must always grant");
  });
});

// ── Already-completed v2 key returns true without re-writing ─────────────────

test("migrate_already_completed_pubkey_returns_true_immediately", () => {
  withFakeWindow({ [V2_KEY]: "true" }, (storage) => {
    // Pass mismatched community to prove early-exit, not community vouching.
    const result = migrateMachineOnboardingCompletion(
      PUBKEY_A,
      PUBKEY_B,
      false,
    );
    assert.equal(result, true, "already-completed pubkey must return true");
    // Value was already there; the function should not have touched it
    // (but a redundant write is also acceptable — just verify it's still true).
    assert.equal(storage.getItem(V2_KEY), "true");
  });
});
