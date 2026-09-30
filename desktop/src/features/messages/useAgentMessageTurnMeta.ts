import * as React from "react";
import { useQuery } from "@tanstack/react-query";

import { buildTranscriptState } from "@/features/agents/ui/agentSessionTranscript";
import {
  mergeObserverEventWindows,
  scopeByChannel,
} from "@/features/agents/ui/agentSessionPanelLayout";
import type {
  ObserverEvent,
  TranscriptItem,
} from "@/features/agents/ui/agentSessionTypes";
import {
  useArchivedChannelEvents,
  useLoadArchivedObserverEvents,
  useObserverEvents,
} from "@/features/agents/ui/useObserverEvents";
import {
  collectThinkingContentItems,
  collectTurnPromptContext,
  extractTriggeringEventIds,
  findNearestTurnIdByTime,
  findTurnIdForPromptEvent,
  findTurnStartedEvent,
  formatTurnDuration,
  formatTurnTokens,
  normalizeUnixSeconds,
  resolveTurnDurationSeconds,
  summarizeThinkingItems,
} from "@/features/messages/lib/agentMessageTurnMeta";
import { useShowAgentThinking } from "@/features/messages/lib/showAgentThinkingPreference";
import {
  getAgentTurnMetricNear,
  type AgentTurnMetricNear,
} from "@/shared/api/tauriArchive";
import { getEventById } from "@/shared/api/tauriEvents";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type AgentMessageTurnMeta = {
  enabled: boolean;
  durationLabel: string | null;
  durationSeconds: number | null;
  tokensLabel: string | null;
  totalTokens: string | null;
  metric: AgentTurnMetricNear | null;
  turnId: string | null;
  /** Thought + tool items only (never lifecycle-only filler). */
  turnItems: TranscriptItem[];
  thoughtCount: number;
  toolCount: number;
  thoughtPreview: string | null;
  hasThinkingContent: boolean;
  joinMethod: "triggering-event" | "time-proximity" | "none";
  metricLoading: boolean;
  durationLoading: boolean;
  thinkingLoading: boolean;
  /** Sections for the shared Prompt context dialog (activity-feed CheckCheck). */
  promptContextSections: Array<{ title: string; body: string }>;
  promptSetupItems: TranscriptItem[];
  hasPromptContext: boolean;
};

/**
 * Best-available join of duration / tokens / thinking for one agent reply.
 *
 * - Duration: triggering prompt / turn_started → reply (not stale parentId)
 * - Tokens: archived 44200 via exact `(sessionId, turnId)` (or unique turnId);
 *   else a tight high-confidence time window. Ambiguous matches are hidden.
 *   Kind 44200 still has no chat message id — that is the remaining schema gap.
 * - Thinking: observer frames for agent+channel; prefer turn whose
 *   `triggeringEventIds` includes `parentId`, else nearest by time that has
 *   thought/tool content. Includes null-turnId chunks in the prompt→reply window.
 */
export function useAgentMessageTurnMeta(input: {
  agentPubkey: string | null | undefined;
  channelId: string | null | undefined;
  messageId: string;
  createdAt: number;
  parentId: string | null | undefined;
  isAgent: boolean;
}): AgentMessageTurnMeta {
  const showThinking = useShowAgentThinking();
  const enabled =
    showThinking &&
    input.isAgent &&
    Boolean(input.agentPubkey) &&
    !Number.isNaN(input.createdAt);

  const agentPubkey = input.agentPubkey
    ? normalizePubkey(input.agentPubkey)
    : null;

  const parentQuery = useQuery({
    queryKey: ["agent-message-parent", input.parentId ?? null],
    enabled: enabled && Boolean(input.parentId),
    staleTime: 60_000,
    queryFn: async () => {
      if (!input.parentId) return null;
      try {
        return await getEventById(input.parentId);
      } catch {
        return null;
      }
    },
  });

  // Live observer frames (cheap; already subscribed for running / community bots).
  const { events: liveEvents } = useObserverEvents(
    enabled && Boolean(agentPubkey),
    agentPubkey,
  );

  // Eager archive hydrate so Thought chip visibility and expand content are
  // available without waiting for the user to open the card. Same path the
  // activity pane uses (owner_p 24200 → channel index → decrypt → store).
  const { hasOlderArchived } = useLoadArchivedObserverEvents(
    enabled && Boolean(input.channelId),
    input.channelId ?? null,
  );

  const archivedEvents = useArchivedChannelEvents(
    agentPubkey,
    input.channelId ?? null,
  );

  const combinedEvents = React.useMemo(() => {
    if (!enabled || !agentPubkey) return [] as ObserverEvent[];
    const scopedLive = scopeByChannel(liveEvents, input.channelId ?? null);
    // Frames with null channelId are dropped by scopeByChannel. Re-include
    // recent null-channel frames from live so early turn chunks still join.
    const nullChannelLive =
      input.channelId != null
        ? liveEvents.filter((event) => {
            if (event.channelId != null) return false;
            if (!event.timestamp) return false;
            const ts = Date.parse(event.timestamp);
            if (!Number.isFinite(ts)) return false;
            return Math.abs(ts / 1000 - input.createdAt) <= 600;
          })
        : [];
    return mergeObserverEventWindows(
      [...scopedLive, ...nullChannelLive],
      archivedEvents,
    );
  }, [
    enabled,
    agentPubkey,
    liveEvents,
    archivedEvents,
    input.channelId,
    input.createdAt,
  ]);

  const transcriptItems = React.useMemo(
    () => buildTranscriptState(combinedEvents).items,
    [combinedEvents],
  );

  const { turnId, sessionId, joinMethod, turnStarted } = React.useMemo(() => {
    const byTrigger = findTurnIdForPromptEvent(combinedEvents, input.parentId);
    if (byTrigger) {
      const started = findTurnStartedEvent(combinedEvents, byTrigger);
      return {
        turnId: byTrigger,
        sessionId: started?.sessionId ?? null,
        joinMethod: "triggering-event" as const,
        turnStarted: started,
      };
    }
    const byTime = findNearestTurnIdByTime(transcriptItems, input.createdAt);
    if (byTime) {
      const started = findTurnStartedEvent(combinedEvents, byTime);
      return {
        turnId: byTime,
        sessionId: started?.sessionId ?? null,
        joinMethod: "time-proximity" as const,
        turnStarted: started,
      };
    }
    return {
      turnId: null,
      sessionId: null,
      joinMethod: "none" as const,
      turnStarted: null,
    };
  }, [combinedEvents, transcriptItems, input.parentId, input.createdAt]);

  // Prefer exact session+turn on 44200; fall back to tight time only.
  const metricQuery = useQuery({
    queryKey: [
      "agent-turn-metric-near",
      agentPubkey,
      input.createdAt,
      sessionId,
      turnId,
    ] as const,
    enabled: enabled && Boolean(agentPubkey),
    staleTime: 30_000,
    queryFn: async () => {
      if (!agentPubkey) return null;
      return getAgentTurnMetricNear({
        agentPubkey,
        aroundSec: input.createdAt,
        windowSec: 180,
        sessionId,
        turnId,
      });
    },
  });

  // Actual harness prompt id for this turn (not necessarily message.parentId —
  // parentId may be an older thread ancestor and inflated duration).
  const triggerPromptId = React.useMemo(() => {
    if (!turnStarted) return null;
    const ids = extractTriggeringEventIds(turnStarted.payload);
    return ids[0] ?? null;
  }, [turnStarted]);

  const promptEventId =
    triggerPromptId ??
    (joinMethod === "triggering-event" ? (input.parentId ?? null) : null);

  const promptQuery = useQuery({
    queryKey: ["agent-message-turn-prompt", promptEventId],
    enabled: enabled && Boolean(promptEventId),
    staleTime: 60_000,
    queryFn: async () => {
      if (!promptEventId) return null;
      try {
        return await getEventById(promptEventId);
      } catch {
        return null;
      }
    },
  });

  // Parent is only a duration fallback when it is the trigger (or we have no
  // turn_started). Still fetched for thinking window bounds.
  const parentCreatedAt = normalizeUnixSeconds(parentQuery.data?.created_at);
  const promptCreatedAt = normalizeUnixSeconds(promptQuery.data?.created_at);

  const turnStartedAtSec = React.useMemo(() => {
    if (!turnStarted?.timestamp) return null;
    const ms = Date.parse(turnStarted.timestamp);
    if (!Number.isFinite(ms)) return null;
    return Math.floor(ms / 1000);
  }, [turnStarted]);

  // Window for collecting thinking: prefer prompt/turn start, not an old parent.
  const thinkingWindowStartSec =
    promptCreatedAt ?? turnStartedAtSec ?? parentCreatedAt;

  const turnItems = React.useMemo(
    () =>
      collectThinkingContentItems(transcriptItems, {
        turnId,
        windowStartSec: thinkingWindowStartSec,
        windowEndSec: input.createdAt,
      }),
    [transcriptItems, turnId, thinkingWindowStartSec, input.createdAt],
  );

  const thinkingSummary = React.useMemo(
    () => summarizeThinkingItems(turnItems),
    [turnItems],
  );

  const hasThinkingContent =
    thinkingSummary.thoughtCount > 0 || thinkingSummary.toolCount > 0;

  const promptContext = React.useMemo(
    () =>
      collectTurnPromptContext(transcriptItems, {
        turnId,
        windowStartSec: thinkingWindowStartSec,
        windowEndSec: input.createdAt,
      }),
    [transcriptItems, turnId, thinkingWindowStartSec, input.createdAt],
  );

  const durationSeconds = React.useMemo(() => {
    // Prefer harness trigger / turn_started; fall back to parentId timestamp so
    // the duration chip always has a candidate when any prompt exists.
    const promptForDuration = promptCreatedAt ?? parentCreatedAt;

    return resolveTurnDurationSeconds({
      replyCreatedAt: input.createdAt,
      turnStartedAtSec,
      promptCreatedAtSec: promptForDuration,
    });
  }, [input.createdAt, promptCreatedAt, parentCreatedAt, turnStartedAtSec]);

  // Show whatever the archive returns — do not hide on matchKind.
  const metric = metricQuery.data ?? null;
  const totalTokens =
    metric?.turnTotalTokens ??
    (metric?.turnInputTokens && metric?.turnOutputTokens
      ? sumTokenStrings(metric.turnInputTokens, metric.turnOutputTokens)
      : (metric?.turnOutputTokens ?? metric?.turnInputTokens ?? null));

  // Archive still paging and we have no thinking yet → treat as loading so
  // the UI does not flash an empty Thought chip / card.
  const thinkingLoading =
    Boolean(input.channelId) &&
    hasOlderArchived !== false &&
    !hasThinkingContent &&
    combinedEvents.length === 0;

  return {
    enabled,
    durationLabel: formatTurnDuration(durationSeconds),
    durationSeconds,
    tokensLabel: formatTurnTokens(totalTokens),
    totalTokens,
    metric,
    turnId,
    turnItems,
    thoughtCount: thinkingSummary.thoughtCount,
    toolCount: thinkingSummary.toolCount,
    thoughtPreview: thinkingSummary.thoughtPreview,
    hasThinkingContent,
    joinMethod,
    metricLoading: metricQuery.isLoading,
    durationLoading:
      (Boolean(promptEventId) && promptQuery.isLoading) ||
      (Boolean(input.parentId) && !promptEventId && parentQuery.isLoading),
    thinkingLoading,
    promptContextSections: promptContext.sections,
    promptSetupItems: promptContext.setup,
    hasPromptContext: promptContext.hasContext,
  };
}

function sumTokenStrings(a: string, b: string): string | null {
  try {
    return (BigInt(a) + BigInt(b)).toString();
  } catch {
    return null;
  }
}
