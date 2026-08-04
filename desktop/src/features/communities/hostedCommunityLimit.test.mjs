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
  hostedCommunityLimitReachedMessage,
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

// ---------------------------------------------------------------------------
// Caller-supplied fallback. Mutation replies (create/transfer) may omit the
// field, so resolving one must never drag an already-known limit back down to
// the stock default.
// ---------------------------------------------------------------------------

test("resolve_uses_caller_fallback_when_field_is_absent", () => {
  assert.equal(resolveHostedCommunityLimit({}, 7), 7);
  assert.equal(resolveHostedCommunityLimit(undefined, 7), 7);
  assert.equal(resolveHostedCommunityLimit({ error: { code: "taken" } }, 3), 3);
});

test("resolve_uses_caller_fallback_on_invalid_values", () => {
  assert.equal(
    resolveHostedCommunityLimit({ max_communities_per_owner: 0 }, 7),
    7,
  );
  assert.equal(
    resolveHostedCommunityLimit({ max_communities_per_owner: "7" }, 7),
    7,
  );
});

test("resolve_prefers_server_value_over_caller_fallback", () => {
  assert.equal(
    resolveHostedCommunityLimit({ max_communities_per_owner: 9 }, 7),
    9,
  );
  assert.equal(
    resolveHostedCommunityLimit({ max_communities_per_owner: 2 }, 7),
    2,
  );
});

test("mutation_reply_without_the_field_keeps_the_loaded_limit", () => {
  // A relay reporting 7 at load time, then a create rejection that carries no
  // limit: the copy and gate must stay on 7, not silently revert to 5.
  const loadedLimit = resolveHostedCommunityLimit({
    max_communities_per_owner: 7,
  });
  const rejection = { error: { code: "limit_reached" } };
  assert.equal(resolveHostedCommunityLimit(rejection, loadedLimit), 7);
});

// ---------------------------------------------------------------------------
// The `limit_reached` copy must never invent a number: without a
// server-resolved limit it omits the count instead of asserting 5.
// ---------------------------------------------------------------------------

test("limit_reached_copy_names_the_resolved_limit", () => {
  assert.equal(
    hostedCommunityLimitReachedMessage(7),
    "You've reached the limit of 7 hosted communities.",
  );
  assert.equal(
    hostedCommunityLimitReachedMessage(3),
    "You've reached the limit of 3 hosted communities.",
  );
});

test("limit_reached_copy_omits_the_number_when_unresolved", () => {
  for (const unresolved of [undefined, 0]) {
    const message = hostedCommunityLimitReachedMessage(unresolved);
    assert.equal(message, "You've reached your limit of hosted communities.");
    assert.equal(
      /\d/.test(message),
      false,
      `copy must not fabricate a limit: ${message}`,
    );
  }
});
