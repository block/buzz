/**
 * Unit tests for the hosted-community per-owner limit resolver (#4160).
 *
 * The relay derives the effective limit from BUZZ_MAX_COMMUNITIES_PER_OWNER
 * (crates/buzz-db/src/relay_members.rs) and exposes it on operator wire
 * responses as `max_communities_per_owner`. The desktop must consume that
 * value instead of hardcoding 5 — these tests pin the resolver's fallback
 * rules (mirroring the relay's effective_owner_limit) and the gating
 * regression that motivated the fix.
 */
import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_HOSTED_COMMUNITY_LIMIT,
  resolveHostedCommunityLimit,
} from "./hostedCommunityLimit.ts";

// ---------------------------------------------------------------------------
// Fallback cases — mirror the relay's effective_owner_limit rules: a missing,
// non-numeric, non-integer, or non-positive value falls back to the default.
// ---------------------------------------------------------------------------

test("default_limit_is_five", () => {
  // The stock default must stay in lockstep with MAX_COMMUNITIES_PER_OWNER
  // in crates/buzz-db/src/relay_members.rs.
  assert.equal(DEFAULT_HOSTED_COMMUNITY_LIMIT, 5);
});

test("resolve_falls_back_when_response_is_absent", () => {
  assert.equal(resolveHostedCommunityLimit(undefined), 5);
  assert.equal(resolveHostedCommunityLimit(null), 5);
});

test("resolve_falls_back_when_field_is_absent", () => {
  assert.equal(resolveHostedCommunityLimit({}), 5);
  assert.equal(resolveHostedCommunityLimit({ communities: [] }), 5);
});

test("resolve_falls_back_on_invalid_values", () => {
  assert.equal(
    resolveHostedCommunityLimit({ max_communities_per_owner: 0 }),
    5,
  );
  assert.equal(
    resolveHostedCommunityLimit({ max_communities_per_owner: -5 }),
    5,
  );
  assert.equal(
    resolveHostedCommunityLimit({ max_communities_per_owner: 2.5 }),
    5,
  );
  assert.equal(
    resolveHostedCommunityLimit({ max_communities_per_owner: Number.NaN }),
    5,
  );
  assert.equal(
    resolveHostedCommunityLimit({
      max_communities_per_owner: Number.POSITIVE_INFINITY,
    }),
    5,
  );
  assert.equal(
    // Untyped IPC payloads can smuggle strings; a string "7" is not a limit.
    resolveHostedCommunityLimit({ max_communities_per_owner: "7" }),
    5,
  );
});

// ---------------------------------------------------------------------------
// Server-provided values win.
// ---------------------------------------------------------------------------

test("resolve_uses_server_value_above_default", () => {
  assert.equal(
    resolveHostedCommunityLimit({ max_communities_per_owner: 7 }),
    7,
  );
  assert.equal(
    resolveHostedCommunityLimit({ max_communities_per_owner: 100 }),
    100,
  );
});

test("resolve_uses_server_value_below_default", () => {
  assert.equal(
    resolveHostedCommunityLimit({ max_communities_per_owner: 3 }),
    3,
  );
  assert.equal(
    resolveHostedCommunityLimit({ max_communities_per_owner: 1 }),
    1,
  );
});

// ---------------------------------------------------------------------------
// The regression that IS the bug (#4160): the UI gate `owned >= limit` must
// track the server-reported limit in both directions, not the hardcoded 5.
// ---------------------------------------------------------------------------

test("owner_of_five_is_not_gated_when_server_limit_is_seven", () => {
  const limit = resolveHostedCommunityLimit({ max_communities_per_owner: 7 });
  const ownedCommunities = 5;
  assert.equal(ownedCommunities >= limit, false);
});

test("owner_of_three_is_gated_when_server_limit_is_three", () => {
  const limit = resolveHostedCommunityLimit({ max_communities_per_owner: 3 });
  const ownedCommunities = 3;
  assert.equal(ownedCommunities >= limit, true);
});

test("owner_of_five_is_gated_at_stock_default", () => {
  const limit = resolveHostedCommunityLimit({});
  const ownedCommunities = 5;
  assert.equal(ownedCommunities >= limit, true);
});
