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
  /** Latest observed activity timestamp, keyed by channel id. */
  latestActivityAtByChannel: Record<string, number>;
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
    latestActivityByChannelEqual(
      left.latestActivityAtByChannel,
      right.latestActivityAtByChannel,
    ) &&
    channelIdsEqual(left.channelIds, right.channelIds)
  );
}

function latestActivityByChannelEqual(
  left: Record<string, number>,
  right: Record<string, number>,
): boolean {
  const leftKeys = Object.keys(left);
  const rightKeys = Object.keys(right);
  if (leftKeys.length !== rightKeys.length) {
    return false;
  }

  for (const key of leftKeys) {
    if (left[key] !== right[key]) {
      return false;
    }
  }

  return true;
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

function parseTimestampMillis(timestamp: string): number | null {
  const millis = Date.parse(timestamp);
  return Number.isNaN(millis) ? null : millis;
}

function collectLatestActivityAtByChannel({
  activeTurns,
  events,
  transcript,
}: {
  activeTurns: readonly ActiveTurnSummary[];
  events: readonly ObserverEvent[];
  transcript: readonly TranscriptItem[];
}): Record<string, number> {
  const latestActivityAtByChannel: Record<string, number> = {};

  const record = (channelId: string | null | undefined, timestamp: number) => {
    if (!channelId) {
      return;
    }
    const previous = latestActivityAtByChannel[channelId];
    if (previous === undefined || timestamp > previous) {
      latestActivityAtByChannel[channelId] = timestamp;
    }
  };

  for (const turn of activeTurns) {
    record(turn.channelId, turn.anchorAt);
  }

  for (const event of events) {
    const timestamp = parseTimestampMillis(event.timestamp);
    if (timestamp !== null) {
      record(event.channelId, timestamp);
    }
  }

  for (const item of transcript) {
    const timestamp = parseTimestampMillis(item.timestamp);
    if (timestamp !== null) {
      record(item.channelId, timestamp);
    }
  }

  return latestActivityAtByChannel;
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
  const latestActivityAtByChannel = collectLatestActivityAtByChannel({
    activeTurns,
    events,
    transcript,
  });

  if (isLive) {
    const channelIds = [...activeTurns]
      .map((turn) => turn.channelId)
      .sort((left, right) => left.localeCompare(right));

    return {
      channelIds,
      hasFeedContent: true,
      isLive: true,
      latestActivityAtByChannel,
      preferredChannelId: channelIds[0] ?? null,
    };
  }

  const feedChannelIds = collectChannelIdsFromFeed(events, transcript);
  const latestChannelId = deriveLatestChannelId(events, transcript);

  return {
    channelIds: feedChannelIds,
    hasFeedContent,
    isLive: false,
    latestActivityAtByChannel,
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
