import * as React from "react";

import { getIdentity } from "@/shared/api/tauriIdentity";
import { markCommunityRead } from "@/features/communities/communityMarkRead";
import { pollCommunityUnread } from "@/features/communities/communityUnreadObserver";

import type { Community } from "./types";

const COMMUNITY_UNREAD_POLL_MS = 30_000;

/**
 * Per-community unread summary for the community rail.
 *
 * `state` distinguishes "observed, no unread" from "not observed yet" so the
 * rail never renders a false "no unread" for a relay it could not reach:
 * - `unknown`  — not yet observed (render dim, no unread affordance)
 * - `loading`  — observation in flight (render dim/skeleton)
 * - `ready`    — observed; trust `hasUnread` / `count`
 * - `error`    — observation failed (render neutral, never "no unread")
 *
 * `count` carries the MENTION count (not total unread) — the rail shows a dot
 * for any unread and a numeric badge only when mentions are present.
 */
export type CommunityUnreadState = {
  hasUnread: boolean;
  count?: number;
  state: "unknown" | "loading" | "ready" | "error";
};

const unknownUnreadState: CommunityUnreadState = {
  hasUnread: false,
  state: "unknown",
};

function seedCommunityStates(
  communities: Community[],
  previous: Record<string, CommunityUnreadState>,
): Record<string, CommunityUnreadState> {
  const next: Record<string, CommunityUnreadState> = {};
  for (const community of communities) {
    next[community.id] = previous[community.id] ?? unknownUnreadState;
  }
  return next;
}

/**
 * Observe unread activity for INACTIVE communities without touching the active
 * relay singleton.
 */
export function useCommunityUnread(
  communities: Community[],
  activeCommunityId: string | null,
): {
  unreadByCommunity: Record<string, CommunityUnreadState>;
  markCommunityRead: (communityId: string) => Promise<void>;
} {
  const [unreadByCommunity, setUnreadByCommunity] = React.useState<
    Record<string, CommunityUnreadState>
  >(() => seedCommunityStates(communities, {}));

  React.useEffect(() => {
    let cancelled = false;
    let pollTimer: number | null = null;
    const inactiveCommunities = communities.filter(
      (community) => community.id !== activeCommunityId,
    );

    setUnreadByCommunity((previous) =>
      seedCommunityStates(communities, previous),
    );

    const markLoading = (communityId: string) => {
      setUnreadByCommunity((previous) => {
        const current = previous[communityId] ?? unknownUnreadState;
        if (current.state === "ready") {
          return previous;
        }
        return {
          ...previous,
          [communityId]: { hasUnread: false, state: "loading" },
        };
      });
    };

    const markReady = (
      communityId: string,
      result: { hasUnread: boolean; mentionCount: number },
    ) => {
      setUnreadByCommunity((previous) => ({
        ...previous,
        [communityId]: {
          hasUnread: result.hasUnread,
          count: result.mentionCount > 0 ? result.mentionCount : undefined,
          state: "ready",
        },
      }));
    };

    const markError = (communityId: string) => {
      setUnreadByCommunity((previous) => ({
        ...previous,
        [communityId]: { hasUnread: false, state: "error" },
      }));
    };

    const scheduleNextPoll = () => {
      if (cancelled) return;
      pollTimer = window.setTimeout(() => {
        void pollInactiveCommunities();
      }, COMMUNITY_UNREAD_POLL_MS);
    };

    const pollInactiveCommunities = async () => {
      if (inactiveCommunities.length === 0) {
        return;
      }

      let pubkey: string;
      try {
        pubkey = (await getIdentity()).pubkey;
      } catch {
        for (const community of inactiveCommunities) {
          if (cancelled) return;
          markError(community.id);
        }
        scheduleNextPoll();
        return;
      }

      for (const community of inactiveCommunities) {
        if (cancelled) return;
        markLoading(community.id);
        try {
          const result = await pollCommunityUnread(community, pubkey);
          if (cancelled) return;
          markReady(community.id, result);
        } catch (error) {
          console.debug(
            `[CommunityUnread] poll failed community=${community.id}:`,
            error,
          );
          if (cancelled) return;
          markError(community.id);
        }
      }

      scheduleNextPoll();
    };

    void pollInactiveCommunities();

    return () => {
      cancelled = true;
      if (pollTimer !== null) {
        window.clearTimeout(pollTimer);
      }
    };
  }, [activeCommunityId, communities]);

  const communitiesRef = React.useRef(communities);
  communitiesRef.current = communities;

  const markRead = React.useCallback(
    async (communityId: string) => {
      const community = communitiesRef.current.find(
        (candidate) => candidate.id === communityId,
      );
      if (!community || communityId === activeCommunityId) return;

      const { pubkey } = await getIdentity();
      await markCommunityRead(community, pubkey);
      // Optimistic clear — the next poll re-verifies against the relay.
      setUnreadByCommunity((previous) => ({
        ...previous,
        [communityId]: { hasUnread: false, state: "ready" },
      }));
    },
    [activeCommunityId],
  );

  return { unreadByCommunity, markCommunityRead: markRead };
}
