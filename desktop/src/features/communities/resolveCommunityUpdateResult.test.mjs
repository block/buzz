/**
 * Unit tests for the updateCommunity result matrix (Phase 1).
 * Tests the pure decision logic extracted into resolveCommunityUpdateResult.
 */
import assert from "node:assert/strict";
import test from "node:test";

import { resolveCommunityUpdateResult } from "./useCommunities.tsx";

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
