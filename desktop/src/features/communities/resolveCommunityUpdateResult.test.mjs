/**
 * Unit tests for the updateCommunity result matrix (Phase 1).
 * Tests the pure decision logic extracted into resolveCommunityUpdateResult.
 */
import assert from "node:assert/strict";
import test from "node:test";

import {
  hasCommunityForRelay,
  isSameRelay,
  resolveCommunityUpdateResult,
} from "./useCommunities.tsx";
import { storageKey } from "@/features/profile/lib/selfProfileStorage";

const COMMUNITIES = [
  {
    id: "ws-1",
    name: "Community A",
    relayUrl: "wss://relay-a.example.com",
    addedAt: "2024-01-01",
  },
  {
    id: "ws-2",
    name: "Community B",
    relayUrl: "wss://relay-b.example.com",
    addedAt: "2024-01-02",
  },
];

// ---------------------------------------------------------------------------
// 5-case matrix from the plan
// ---------------------------------------------------------------------------

test("resolveCommunityUpdateResult_untouched_submit_returns_unchanged", () => {
  // Prefilled overlay submitted with identical values — no persist, no bump.
  const result = resolveCommunityUpdateResult(COMMUNITIES, "ws-1", "ws-1", {
    name: "Community A",
    relayUrl: "wss://relay-a.example.com",
  });
  assert.deepEqual(result, { kind: "unchanged" });
});

test("resolveCommunityUpdateResult_name_only_edit_returns_updated_without_reinit", () => {
  // Name change persists but does NOT trigger a backend reapply.
  const result = resolveCommunityUpdateResult(COMMUNITIES, "ws-1", "ws-1", {
    name: "New Name",
  });
  assert.deepEqual(result, { kind: "updated", requiresReinit: false });
});

test("resolveCommunityUpdateResult_relay_edit_returns_updated_with_reinit", () => {
  // Relay URL change on the active community triggers backend reapply.
  const result = resolveCommunityUpdateResult(COMMUNITIES, "ws-1", "ws-1", {
    relayUrl: "wss://relay-c.example.com",
  });
  assert.deepEqual(result, { kind: "updated", requiresReinit: true });
});

test("resolveCommunityUpdateResult_duplicate_relay_returns_duplicate", () => {
  // Trying to set ws-1's relay to ws-2's relay URL is a duplicate.
  const result = resolveCommunityUpdateResult(COMMUNITIES, "ws-1", "ws-1", {
    relayUrl: "wss://relay-b.example.com",
  });
  assert.deepEqual(result, { kind: "duplicate-relay" });
});

test("resolveCommunityUpdateResult_not_found_returns_not_found", () => {
  const result = resolveCommunityUpdateResult(
    COMMUNITIES,
    "ws-1",
    "ws-nonexistent",
    {
      name: "Whatever",
    },
  );
  assert.deepEqual(result, { kind: "not-found" });
});

// ---------------------------------------------------------------------------
// Additional edge cases
// ---------------------------------------------------------------------------

test("resolveCommunityUpdateResult_relay_edit_on_inactive_community_no_reinit", () => {
  // Relay change on a NON-active community persists but doesn't reinit.
  const result = resolveCommunityUpdateResult(COMMUNITIES, "ws-1", "ws-2", {
    relayUrl: "wss://relay-c.example.com",
  });
  assert.deepEqual(result, { kind: "updated", requiresReinit: false });
});

test("resolveCommunityUpdateResult_token_change_on_active_requires_reinit", () => {
  const result = resolveCommunityUpdateResult(COMMUNITIES, "ws-1", "ws-1", {
    token: "new-token",
  });
  assert.deepEqual(result, { kind: "updated", requiresReinit: true });
});

test("resolveCommunityUpdateResult_pubkey_change_does_not_require_reinit", () => {
  // pubkey is display-only — not a backend-relevant field.
  const result = resolveCommunityUpdateResult(COMMUNITIES, "ws-1", "ws-1", {
    pubkey: "newpubkey123",
  });
  assert.deepEqual(result, { kind: "updated", requiresReinit: false });
});

test("resolveCommunityUpdateResult_same_relay_url_is_not_duplicate_of_self", () => {
  // Setting the same relay URL that ws-1 already has is unchanged, not duplicate.
  const result = resolveCommunityUpdateResult(COMMUNITIES, "ws-1", "ws-1", {
    relayUrl: "wss://relay-a.example.com",
  });
  assert.deepEqual(result, { kind: "unchanged" });
});

// ---------------------------------------------------------------------------
// Relay identity is the storage notion of sameness, not string equality
// ---------------------------------------------------------------------------
//
// Every per-relay slot — self profile, read state, channel sections, sort
// preference, sidebar watermark, thread activity, observed unread — is keyed
// through `normalizeRelayUrl` (trim, strip trailing slashes, lowercase). Two
// communities whose URLs normalize alike therefore already share all of that
// data, so admitting the second one does not separate them, it hides that
// they are one.

test("isSameRelay_treatsStorageEquivalentSpellingsAsOneRelay", () => {
  const canonical = "wss://relay-a.example.com";
  for (const spelling of [
    "wss://relay-a.example.com/",
    "wss://relay-a.example.com//",
    "WSS://Relay-A.Example.com",
    "  wss://relay-a.example.com  ",
  ]) {
    assert.equal(
      isSameRelay(canonical, spelling),
      true,
      `${spelling} keys to the same storage slot and must be the same relay`,
    );
    assert.equal(
      storageKey(canonical, "pk") === storageKey(spelling, "pk"),
      true,
      `${spelling} must actually collide in storage — the premise of this test`,
    );
  }
});

test("isSameRelay_keepsGenuinelyDifferentRelaysApart", () => {
  assert.equal(
    isSameRelay("wss://relay-a.example.com", "wss://relay-b.example.com"),
    false,
  );
  assert.equal(
    isSameRelay("wss://relay-a.example.com", "ws://relay-a.example.com"),
    false,
  );
});

test("resolveCommunityUpdateResult_trailingSlashOfAnotherRelay_isADuplicate", () => {
  // Before: raw === missed this, so the edit was accepted and the two
  // communities silently shared every per-relay storage slot.
  const result = resolveCommunityUpdateResult(COMMUNITIES, "ws-1", "ws-1", {
    relayUrl: "wss://relay-b.example.com/",
  });
  assert.deepEqual(result, { kind: "duplicate-relay" });
});

test("resolveCommunityUpdateResult_caseOnlyEditOfAnotherRelay_isADuplicate", () => {
  const result = resolveCommunityUpdateResult(COMMUNITIES, "ws-1", "ws-1", {
    relayUrl: "WSS://Relay-B.Example.com",
  });
  assert.deepEqual(result, { kind: "duplicate-relay" });
});

test("resolveCommunityUpdateResult_reSpellingOwnRelay_isStillAnUpdate", () => {
  // Editing your own relay to an equivalent spelling is not a duplicate of
  // yourself — it stays a normal update, and on the active community it still
  // reinitialises the backend, because the connection URL really did change.
  const result = resolveCommunityUpdateResult(COMMUNITIES, "ws-1", "ws-1", {
    relayUrl: "wss://relay-a.example.com/",
  });
  assert.deepEqual(result, { kind: "updated", requiresReinit: true });
});

// ---------------------------------------------------------------------------
// The onboarding rollback flag must agree with addCommunity
// ---------------------------------------------------------------------------
//
// `handleCommunityOnboardingConnect` records `addedCommunity` from this
// predicate and `handleCommunityOnboardingCancel` acts on it: a true flag lets
// cancel remove the community, or `clearCommunities()` it when it is the only
// one. Since `addCommunity` folds an equivalent spelling into the existing
// community rather than creating a new one, asking with raw `===` here would
// arm that rollback against a community the connect never created.

test("hasCommunityForRelay_reportsAnEquivalentSpellingAsAlreadyPresent", () => {
  for (const spelling of [
    "wss://relay-a.example.com/",
    "wss://relay-a.example.com//",
    "WSS://Relay-A.Example.com",
    "  wss://relay-a.example.com  ",
  ]) {
    assert.equal(
      hasCommunityForRelay(COMMUNITIES, spelling),
      true,
      `${spelling} is the existing relay, so the connect added nothing`,
    );
  }
});

test("hasCommunityForRelay_reportsAGenuinelyNewRelayAsAbsent", () => {
  assert.equal(
    hasCommunityForRelay(COMMUNITIES, "wss://relay-c.example.com"),
    false,
  );
  assert.equal(
    hasCommunityForRelay(COMMUNITIES, "ws://relay-a.example.com"),
    false,
  );
  assert.equal(hasCommunityForRelay([], "wss://relay-a.example.com"), false);
});

test("hasCommunityForRelay_agreesWithTheMatchAddCommunityUses", () => {
  // addCommunity folds via isSameRelay; the rollback flag must not disagree
  // with it for any spelling, or cancel deletes a pre-existing community.
  for (const spelling of [
    "wss://relay-a.example.com",
    "WSS://Relay-A.Example.com/",
    "wss://relay-b.example.com//",
    "wss://relay-c.example.com",
  ]) {
    assert.equal(
      hasCommunityForRelay(COMMUNITIES, spelling),
      COMMUNITIES.some((community) =>
        isSameRelay(community.relayUrl, spelling),
      ),
      `${spelling} must resolve the same way for both`,
    );
  }
});
