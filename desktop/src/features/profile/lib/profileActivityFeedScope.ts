import * as React from "react";

import type { ActiveTurnSummary } from "@/features/agents/activeAgentTurnsStore";
import { subscribeActiveAgentTurns } from "@/features/agents/activeAgentTurnsStore";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import {
  getAgentObserverSnapshot,
  getAgentTranscript,
  subscribeAgentObserverStore,
} from "@/features/agents/observerRelayStore";
import type {
  ObserverEvent,
  TranscriptItem,
} from "@/features/agents/ui/agentSessionTypes";
import type { ProfileActivityAgent } from "@/features/profile/lib/profileActivityAgent";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type ProfileActivityFeedScope = {
  /** Distinct channel ids to surface in the embed switcher. */
  channelIds: string[];
  /** Whether the observer feed has any events or transcript for this agent. */
  hasFeedContent: boolean;
  /** True while the active-turn store reports live work for this agent. */
  isLive: boolean;
  /** Preferred channel scope when no explicit selection exists yet. */
  preferredChannelId: string | null;
};

const cachedScopes = new Map<string, ProfileActivityFeedScope>();

function channelIdsEqual(
  left: readonly string[],
  right: readonly string[],
): boolean {
  if (left.length !== right.length) {
    return false;
  }

  for (let index = 0; index < left.length; index += 1) {
    if (left[index] !== right[index]) {
      return false;
    }
  }

  return true;
}

function scopesEqual(
  left: ProfileActivityFeedScope,
  right: ProfileActivityFeedScope,
): boolean {
  return (
    left.hasFeedContent === right.hasFeedContent &&
    left.isLive === right.isLive &&
    left.preferredChannelId === right.preferredChannelId &&
    channelIdsEqual(left.channelIds, right.channelIds)
  );
}

function stableFeedScope(
  cacheKey: string,
  next: ProfileActivityFeedScope,
): ProfileActivityFeedScope {
  const cached = cachedScopes.get(cacheKey);
  if (cached && scopesEqual(cached, next)) {
    return cached;
  }

  cachedScopes.set(cacheKey, next);
  return next;
}

function collectChannelIdsFromFeed(
  events: readonly ObserverEvent[],
  transcript: readonly TranscriptItem[],
): string[] {
  const channelIds = new Set<string>();
  for (const event of events) {
    if (event.channelId) {
      channelIds.add(event.channelId);
    }
  }
  for (const item of transcript) {
    if (item.channelId) {
      channelIds.add(item.channelId);
    }
  }
  return [...channelIds].sort((left, right) => left.localeCompare(right));
}

function deriveLatestChannelId(
  events: readonly ObserverEvent[],
  transcript: readonly TranscriptItem[],
): string | null {
  for (let index = transcript.length - 1; index >= 0; index -= 1) {
    const channelId = transcript[index]?.channelId;
    if (channelId) {
      return channelId;
    }
  }

  for (let index = events.length - 1; index >= 0; index -= 1) {
    const channelId = events[index]?.channelId;
    if (channelId) {
      return channelId;
    }
  }

  return null;
}

export function deriveProfileActivityFeedScope({
  activeTurns,
  events,
  transcript,
}: {
  activeTurns: readonly ActiveTurnSummary[];
  events: readonly ObserverEvent[];
  transcript: readonly TranscriptItem[];
}): ProfileActivityFeedScope {
  const hasFeedContent = events.length > 0 || transcript.length > 0;
  const isLive = activeTurns.length > 0;

  if (isLive) {
    const channelIds = [...activeTurns]
      .map((turn) => turn.channelId)
      .sort((left, right) => left.localeCompare(right));

    return {
      channelIds,
      hasFeedContent: true,
      isLive: true,
      preferredChannelId: channelIds[0] ?? null,
    };
  }

  const feedChannelIds = collectChannelIdsFromFeed(events, transcript);
  const latestChannelId = deriveLatestChannelId(events, transcript);

  return {
    channelIds: feedChannelIds,
    hasFeedContent,
    isLive: false,
    preferredChannelId: latestChannelId,
  };
}

export function useProfileActivityFeedScope(
  activityAgent: ProfileActivityAgent | null,
  activeTurns: readonly ActiveTurnSummary[],
): ProfileActivityFeedScope {
  const agentCacheKey = activityAgent
    ? normalizePubkey(activityAgent.pubkey)
    : "none";
  const hasObserver =
    activityAgent !== null && isManagedAgentActive(activityAgent);

  const getSnapshot = React.useCallback(() => {
    if (!activityAgent || !hasObserver) {
      return stableFeedScope(
        agentCacheKey,
        deriveProfileActivityFeedScope({
          activeTurns,
          events: [],
          transcript: [],
        }),
      );
    }

    const { events } = getAgentObserverSnapshot(activityAgent.pubkey, true);
    const transcript = getAgentTranscript(activityAgent.pubkey, true);
    return stableFeedScope(
      agentCacheKey,
      deriveProfileActivityFeedScope({ activeTurns, events, transcript }),
    );
  }, [activeTurns, activityAgent, agentCacheKey, hasObserver]);

  const snapshot = React.useSyncExternalStore((onStoreChange) => {
    const unsubscribeObserver = subscribeAgentObserverStore(onStoreChange);
    const unsubscribeTurns = subscribeActiveAgentTurns(onStoreChange);
    return () => {
      unsubscribeObserver();
      unsubscribeTurns();
    };
  }, getSnapshot);

  return snapshot;
}
