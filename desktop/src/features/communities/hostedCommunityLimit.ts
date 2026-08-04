/**
 * Pure resolution of the hosted-community per-owner limit (#4160).
 *
 * The relay is the single authority for this limit: it derives the effective
 * value from `BUZZ_MAX_COMMUNITIES_PER_OWNER` (see `max_communities_per_owner`
 * in `crates/buzz-db/src/relay_members.rs`) and exposes it on operator wire
 * responses as `max_communities_per_owner`. The desktop must consume that
 * value rather than hardcode its own copy — otherwise gates and copy drift in
 * both directions the moment a deployment overrides the default.
 *
 * This module stays free of `@tauri-apps` imports (and any other value
 * imports) so `node:test` can load it directly.
 */

/**
 * Fallback used only when a server response does not carry a usable
 * `max_communities_per_owner` — e.g. a relay or intermediary that predates
 * the field. Mirrors `MAX_COMMUNITIES_PER_OWNER` in
 * `crates/buzz-db/src/relay_members.rs`; change the two together.
 */
export const DEFAULT_HOSTED_COMMUNITY_LIMIT = 5;

/**
 * Any response that may carry the relay-reported effective limit. Structural
 * on purpose: importing the response types from `hostedCommunityApi.ts` would
 * pull `@tauri-apps/api/core` into node tests and cycle with that module's
 * re-export of this one.
 */
type LimitBearingResponse = {
  max_communities_per_owner?: unknown;
};

/**
 * Resolve the effective hosted-community limit from a server response.
 *
 * A positive-integer `max_communities_per_owner` wins; anything else (absent
 * response, absent field, non-number, non-integer, or non-positive value)
 * falls back to {@link DEFAULT_HOSTED_COMMUNITY_LIMIT} — the same fallback
 * rules the relay applies to its own env override.
 */
export function resolveHostedCommunityLimit(
  response: LimitBearingResponse | null | undefined,
): number {
  const limit = response?.max_communities_per_owner;
  return typeof limit === "number" && Number.isInteger(limit) && limit > 0
    ? limit
    : DEFAULT_HOSTED_COMMUNITY_LIMIT;
}
