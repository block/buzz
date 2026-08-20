import * as React from "react";

import { useAppShell } from "@/app/AppShellContext";
import { channelCatchUpEventKinds } from "@/features/channels/useUnreadChannels";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useHomeFeedQuery } from "@/features/home/hooks";
import { buildInboxItems } from "@/features/home/lib/inbox";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import {
  candidatesFromInboxItems,
  collectChannelCandidates,
  mergeCandidates,
  type TriageCandidate,
} from "@/features/triage/lib/collectCandidates";
import { useIdentityQuery } from "@/shared/api/hooks";
import { relayClient } from "@/shared/api/relayClient";
import type { FeedItem, HomeFeedResponse } from "@/shared/api/types";

export type TriageCollection = {
  candidates: TriageCandidate[];
  inboxCount: number;
  channelCount: number;
};

/** Mirrors the merge `HomeScreen` performs so both surfaces see one feed. */
function augmentFeed(
  feed: HomeFeedResponse | undefined,
  threadActivityFeedItems: readonly FeedItem[],
): HomeFeedResponse | undefined {
  if (!feed) return undefined;
  if (threadActivityFeedItems.length === 0) return feed;

  return {
    ...feed,
    feed: {
      ...feed.feed,
      activity: [...feed.feed.activity, ...threadActivityFeedItems],
    },
  };
}

/**
 * Assembles the triage scan payload from the two layers described in the PoC:
 * the pre-filtered Home inbox, plus raw per-channel catch-up so the agent also
 * sees the chatter the inbox deliberately hides.
 *
 * Read-state getters come from `useAppShell` rather than `useReadState` — the
 * shell owns the single `ReadStateManager`, and constructing a second one would
 * duplicate its relay subscriptions and read-marker publishes.
 */
export function useTriageCandidates() {
  const identityQuery = useIdentityQuery();
  const currentPubkey = identityQuery.data?.pubkey;
  const channelsQuery = useChannelsQuery();
  const homeFeedQuery = useHomeFeedQuery();
  const {
    getChannelReadAt,
    getMessageReadAt,
    getThreadReadAt,
    threadActivityFeedItems,
  } = useAppShell();

  const channels = channelsQuery.data ?? [];

  const augmentedFeed = React.useMemo(
    () => augmentFeed(homeFeedQuery.data, threadActivityFeedItems),
    [homeFeedQuery.data, threadActivityFeedItems],
  );

  const feedAuthorPubkeys = React.useMemo(() => {
    if (!augmentedFeed) return [];
    const { mentions, needsAction, activity, agentActivity } =
      augmentedFeed.feed;
    return [
      ...new Set(
        [...mentions, ...needsAction, ...activity, ...agentActivity].map(
          (item) => item.pubkey,
        ),
      ),
    ];
  }, [augmentedFeed]);

  const profilesQuery = useUsersBatchQuery(feedAuthorPubkeys);
  const profiles = profilesQuery.data?.profiles;

  const inboxItems = React.useMemo(
    () =>
      buildInboxItems({
        channels,
        currentPubkey,
        feed: augmentedFeed,
        getChannelReadAt,
        getMessageReadAt,
        getThreadReadAt,
        profiles,
      }),
    [
      augmentedFeed,
      channels,
      currentPubkey,
      getChannelReadAt,
      getMessageReadAt,
      getThreadReadAt,
      profiles,
    ],
  );

  const collect = React.useCallback(async (): Promise<TriageCollection> => {
    const context = { currentPubkey, profiles };
    const inbox = candidatesFromInboxItems(inboxItems, context);
    const channelCandidates = await collectChannelCandidates({
      channels,
      context,
      fetchEvents: (filter) => relayClient.fetchEvents(filter),
      getChannelReadAt,
      kindsForChannel: channelCatchUpEventKinds,
    });

    return {
      candidates: mergeCandidates(inbox, channelCandidates),
      inboxCount: inbox.length,
      channelCount: channelCandidates.length,
    };
  }, [channels, currentPubkey, getChannelReadAt, inboxItems, profiles]);

  return {
    collect,
    currentPubkey,
    inboxItems,
    isLoading: channelsQuery.isLoading || homeFeedQuery.isLoading,
  };
}
