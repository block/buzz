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
 * The desktop does not call the relay's operator API directly: every hosted
 * community request goes through Builderlab (`app.builderlab.xyz`, see
 * `desktop/src-tauri/src/builderlab.rs`), which reshapes the relay payload —
 * the desktop sees `id`/`slug`/`normalized_host` where the relay emits
 * `community_id`/`host`. Builderlab must therefore forward
 * `max_communities_per_owner` for a non-default limit to reach these surfaces;
 * until it does, every resolution falls back and the UI behaves exactly as it
 * did before. That fallback is the reason this stays a soft dependency rather
 * than a breaking one.
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
 * Read the limit a server response actually reports, or `null` when it reports
 * none.
 *
 * A positive-integer `max_communities_per_owner` is a limit; anything else
 * (absent response, absent field, non-number, non-integer, or non-positive
 * value) is not — the same rules the relay applies to its own env override.
 *
 * This is the shape to use when a caller already holds a limit and only wants
 * to replace it with a better one (`next ?? previous`): responses that omit the
 * field are indistinguishable from ones that predate it, so they must leave a
 * known-good value alone rather than reset it.
 */
export function readHostedCommunityLimit(
  response: LimitBearingResponse | null | undefined,
): number | null {
  const limit = response?.max_communities_per_owner;
  return typeof limit === "number" && Number.isInteger(limit) && limit > 0
    ? limit
    : null;
}

/**
 * Resolve the effective hosted-community limit from a server response, falling
 * back when it reports none.
 *
 * Pass the limit already in hand as `fallback` when resolving a response that
 * may omit the field (e.g. a mutation reply) so a known-good value from an
 * earlier response is never clobbered by
 * {@link DEFAULT_HOSTED_COMMUNITY_LIMIT}.
 */
export function resolveHostedCommunityLimit(
  response: LimitBearingResponse | null | undefined,
  fallback: number = DEFAULT_HOSTED_COMMUNITY_LIMIT,
): number {
  return readHostedCommunityLimit(response) ?? fallback;
}

/**
 * Whose quota a `limit_reached` rejection is about.
 *
 * The relay rejects a create when the *owner* is at the limit and a transfer
 * when the *transferee* is (`transfer_community` in
 * `crates/buzz-relay/src/api/operator.rs`), but both rejections carry the same
 * `limit_reached:` message prefix and Builderlab collapses them onto a single
 * `limit_reached` code. The requesting call site is therefore the only place
 * that knows which party the number describes — telling an owner who is giving
 * a community away that *they* are out of slots sends them right back to the
 * action that just failed.
 */
export type HostedCommunityLimitSubject = "owner" | "transferee";

/**
 * User-facing copy for a `limit_reached` rejection — the single owner of this
 * sentence, so the surfaces that render it cannot drift apart.
 *
 * Names a number only when the caller resolved one from a server response.
 * There is deliberately no default: a limit this deployment may not use is
 * worse than copy that omits the number.
 */
export function hostedCommunityLimitReachedMessage(
  communityLimit?: number | null,
  subject: HostedCommunityLimitSubject = "owner",
): string {
  if (subject === "transferee") {
    return communityLimit
      ? `That person already owns the limit of ${communityLimit} hosted communities, so they can’t receive another.`
      : "That person already owns their limit of hosted communities, so they can’t receive another.";
  }
  return communityLimit
    ? `You’ve reached the limit of ${communityLimit} hosted communities.`
    : "You’ve reached your limit of hosted communities.";
}
