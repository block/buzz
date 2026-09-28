import * as React from "react";

import { useAgentWorking } from "@/features/agents/agentWorkingSignal";
import { useChannelMessagesQuery } from "@/features/messages/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { Channel } from "@/shared/api/types";

import {
  isBestieThreadSummarizeLive,
  isBestieThreadSummarizePendingStale,
  messageLooksLikeBestieSummarizeCompetingTrigger,
  messageLooksLikeBestieSummarizeTrigger,
  shouldDisableBestieThreadSummarize,
} from "./bestieThreadSummarizeLive";
import {
  abandonBestieThreadSummarizeForScope,
  useBestieThreads,
} from "./bestieThreadStore";
import type { BestieThreadScope } from "./bestieThreadTypes";

/**
 * Summarize live UI for Threads RHS: 🤔… only while the summarize turn is the
 * latest open system turn (not Coffee / Jobs / Reminder).
 */
export function useBestieThreadSummarizeLive(
  scope: BestieThreadScope | null,
  bestieChannel: Channel | null | undefined,
): {
  summarizeDisabled: boolean;
  summarizeLive: boolean;
  summarizeLiveThreadId: string | null;
} {
  const threadState = useBestieThreads(scope);
  const agentPubkey = scope?.agentPubkey ?? null;
  const channelId = bestieChannel?.id ?? null;
  const working = useAgentWorking(agentPubkey, channelId);
  const messagesQuery = useChannelMessagesQuery(bestieChannel ?? null);

  const pendingSummarize = threadState.pendingSummarize;
  const tickWhilePending = pendingSummarize != null;
  const [nowSeconds, setNowSeconds] = React.useState(() =>
    Math.floor(Date.now() / 1000),
  );
  React.useEffect(() => {
    if (!tickWhilePending) return;
    setNowSeconds(Math.floor(Date.now() / 1000));
    const timer = window.setInterval(() => {
      setNowSeconds(Math.floor(Date.now() / 1000));
    }, 1000);
    return () => window.clearInterval(timer);
  }, [tickWhilePending, pendingSummarize?.startedAt]);

  const { latestCompetingTriggerAt, openSummarizeTriggerAt } =
    React.useMemo(() => {
      if (!scope || !agentPubkey) {
        return {
          latestCompetingTriggerAt: null as number | null,
          openSummarizeTriggerAt: null as number | null,
        };
      }
      const owner = normalizePubkey(scope.ownerPubkey);
      const events = messagesQuery.data ?? [];
      const lastSummaryAt = threadState.threads.reduce(
        (max, thread) => Math.max(max, thread.lastSummaryAt ?? 0),
        0,
      );
      let latestSummarize: number | null = null;
      let latestCompeting: number | null = null;
      for (const event of events) {
        if (typeof event.content !== "string") continue;
        if (normalizePubkey(event.pubkey) !== owner) continue;
        const createdAt =
          typeof event.created_at === "number"
            ? event.created_at
            : Math.floor(Date.now() / 1000);
        if (messageLooksLikeBestieSummarizeCompetingTrigger(event.content)) {
          if (latestCompeting == null || createdAt > latestCompeting) {
            latestCompeting = createdAt;
          }
          continue;
        }
        if (!messageLooksLikeBestieSummarizeTrigger(event.content)) continue;
        // Bound pending trigger always counts as open.
        if (pendingSummarize?.triggerMessageId === event.id) {
          if (latestSummarize == null || createdAt > latestSummarize) {
            latestSummarize = createdAt;
          }
          continue;
        }
        // Ignore prompts already folded into a completed summary.
        if (createdAt + 5 < lastSummaryAt) continue;
        if (latestSummarize == null || createdAt > latestSummarize) {
          latestSummarize = createdAt;
        }
      }
      return {
        latestCompetingTriggerAt: latestCompeting,
        openSummarizeTriggerAt: latestSummarize,
      };
    }, [
      agentPubkey,
      messagesQuery.data,
      pendingSummarize?.triggerMessageId,
      scope,
      threadState.threads,
    ]);

  const agentWorkingOnBestieDm = working.working;

  React.useEffect(() => {
    if (!scope) return;
    if (
      !isBestieThreadSummarizePendingStale({
        agentWorkingOnBestieDm,
        nowSeconds,
        pendingSummarize,
        latestCompetingTriggerAt,
      })
    ) {
      return;
    }
    abandonBestieThreadSummarizeForScope(scope);
  }, [
    agentWorkingOnBestieDm,
    latestCompetingTriggerAt,
    nowSeconds,
    pendingSummarize,
    scope,
  ]);

  const summarizeLive = isBestieThreadSummarizeLive({
    agentWorkingOnBestieDm,
    nowSeconds,
    openSummarizeTriggerAt,
    latestCompetingTriggerAt,
    pendingSummarize,
  });
  const summarizeDisabled = shouldDisableBestieThreadSummarize({
    agentWorkingOnBestieDm,
    nowSeconds,
    openSummarizeTriggerAt,
    latestCompetingTriggerAt,
    pendingSummarize,
  });

  return {
    summarizeDisabled,
    summarizeLive,
    summarizeLiveThreadId: summarizeLive
      ? (pendingSummarize?.threadId ?? null)
      : null,
  };
}
