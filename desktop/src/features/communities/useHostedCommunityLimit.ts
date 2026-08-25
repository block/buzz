import * as React from "react";

import type {
  HostedCommunityAccount,
  HostedCommunityMutationResponse,
} from "./hostedCommunityApi";
import {
  DEFAULT_HOSTED_COMMUNITY_LIMIT,
  resolveHostedCommunityLimit,
} from "./hostedCommunityLimit";

/**
 * Owns the hosted-community per-owner limit for one surface (#4160).
 *
 * Every surface that creates or transfers communities needs the same three
 * moves — seed the stock default, apply what a reload reported, adopt what a
 * rejection reported — and all three share one invariant: a response that omits
 * `max_communities_per_owner` is indistinguishable from one that predates the
 * field, so it must leave a known-good value alone (`next ?? previous`) rather
 * than drag it back to {@link DEFAULT_HOSTED_COMMUNITY_LIMIT}. Keeping that
 * rule in a hook means a new surface cannot lose the stickiness by writing a
 * plain `setCommunityLimit(...)`.
 *
 * Lives apart from `hostedCommunityLimit.ts` because that module stays free of
 * value imports so `node:test` can load it directly.
 */
export function useHostedCommunityLimit() {
  const [communityLimit, setCommunityLimit] = React.useState(
    DEFAULT_HOSTED_COMMUNITY_LIMIT,
  );
  // Mirrors the state so the appliers stay referentially stable (they are
  // called from `useCallback` loaders) while still resolving against the limit
  // currently in hand.
  const current = React.useRef(communityLimit);

  const apply = React.useCallback((next: number) => {
    current.current = next;
    setCommunityLimit(next);
    return next;
  }, []);

  /** Apply the limit an account reload reported, keeping the current one if it reported none. */
  const applyFromAccount = React.useCallback(
    (account: HostedCommunityAccount) =>
      apply(account.communityLimit ?? current.current),
    [apply],
  );

  /**
   * Adopt the limit a mutation reply (including a `limit_reached` rejection)
   * reported, and return the effective limit so the caller can render copy
   * against it without waiting for the re-render.
   */
  const adoptFromResponse = React.useCallback(
    (response: HostedCommunityMutationResponse | null | undefined) =>
      apply(resolveHostedCommunityLimit(response, current.current)),
    [apply],
  );

  return { communityLimit, applyFromAccount, adoptFromResponse };
}
