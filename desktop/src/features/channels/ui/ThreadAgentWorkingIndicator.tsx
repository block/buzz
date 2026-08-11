import * as React from "react";

import {
  getAgentTranscript,
  subscribeAgentObserverStore,
} from "@/features/agents/observerRelayStore";
import { TurnLivenessIndicator } from "@/features/agents/ui/TurnLivenessIndicator";
import {
  collectActivityHeadlines,
  latestActivityHeadline,
} from "@/features/agents/ui/agentSessionTranscriptPresentation";
import type { BotActivityAgent } from "@/features/channels/ui/BotActivityBar";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { Shimmer } from "@/shared/ui/Shimmer";
import { UserAvatar } from "@/shared/ui/UserAvatar";

/** How many recent CoT/tool spine lines to stack under the message. */
const COT_STACK_LIMIT = 4;

type ThreadAgentWorkingIndicatorProps = {
  agents: BotActivityAgent[];
  channelId?: string | null;
  className?: string;
  onOpenAgentSession?: (pubkey: string, channelId?: string | null) => void;
  profiles?: UserProfileLookup;
  workingBotPubkeys: string[];
};

export type AgentWorkingStatus = {
  agent: BotActivityAgent;
  /** Live observer headline stack (newest last), empty when only liveness. */
  headlines: string[];
  /** Latest headline, or a thinking fallback for labels. */
  status: string;
};

/** Pure: build per-agent CoT status from current observer transcripts. */
export function buildAgentWorkingStatuses(
  workingAgents: readonly BotActivityAgent[],
  channelId: string | null | undefined,
  readTranscript: (
    pubkey: string,
  ) => readonly import("@/features/agents/ui/agentSessionTypes").TranscriptItem[] = (
    pubkey,
  ) => getAgentTranscript(pubkey, true),
): AgentWorkingStatus[] {
  return workingAgents.map((agent) => {
    const transcript = readTranscript(agent.pubkey);
    const headlines = collectActivityHeadlines(transcript, {
      channelId,
      limit: COT_STACK_LIMIT,
    });
    const latest =
      latestActivityHeadline(transcript, channelId) ??
      headlines[headlines.length - 1] ??
      null;
    return {
      agent,
      headlines,
      status: latest ?? "Thinking",
    };
  });
}

/** Pure: one short status line for aria / multi-agent rotation. */
export function formatAgentWorkingStatusLabel(
  statuses: readonly AgentWorkingStatus[],
  focusIndex = 0,
): string {
  if (statuses.length === 0) {
    return "";
  }
  if (statuses.length === 1) {
    const only = statuses[0];
    const name = only.agent.name || "Agent";
    if (only.status === "Thinking") {
      return `${name} is thinking…`;
    }
    return `${name}: ${only.status}`;
  }

  const focus = statuses[focusIndex % statuses.length] ?? statuses[0];
  const others = statuses.length - 1;
  const name = focus.agent.name || "Agent";
  const focusPart =
    focus.status === "Thinking"
      ? `${name} is thinking…`
      : `${name}: ${focus.status}`;
  return others === 1 ? `${focusPart} · +1 agent` : `${focusPart} · +${others}`;
}

function useWorkingAgentStatuses(
  workingAgents: BotActivityAgent[],
  channelId: string | null | undefined,
): AgentWorkingStatus[] {
  // Fingerprint of stacked headlines so we re-render as CoT advances.
  const fingerprint = React.useSyncExternalStore(
    subscribeAgentObserverStore,
    () =>
      workingAgents
        .map((agent) => {
          const headlines = collectActivityHeadlines(
            getAgentTranscript(agent.pubkey, true),
            { channelId, limit: COT_STACK_LIMIT },
          );
          return `${agent.pubkey.toLowerCase()}=${headlines.join("¦") || "Thinking"}`;
        })
        .join("|"),
  );

  return React.useMemo(() => {
    // Parse stacked headlines from the store fingerprint (newest order preserved).
    const headlinesByPubkey = new Map<string, string[]>();
    for (const part of fingerprint.split("|")) {
      if (!part) continue;
      const eq = part.indexOf("=");
      if (eq <= 0) continue;
      const pk = part.slice(0, eq);
      const raw = part.slice(eq + 1);
      headlinesByPubkey.set(
        pk,
        raw === "Thinking" || raw === "" ? [] : raw.split("¦").filter(Boolean),
      );
    }
    return workingAgents.map((agent) => {
      const headlines = headlinesByPubkey.get(agent.pubkey.toLowerCase()) ?? [];
      return {
        agent,
        headlines,
        status: headlines[headlines.length - 1] ?? "Thinking",
      };
    });
  }, [fingerprint, workingAgents]);
}

/**
 * Live chain-of-thought progress for an in-flight agent turn, rendered under
 * the triggering message. Mirrors the agent session activity spine (tool and
 * thought headlines from the observer transcript) rather than a static
 * "working" bubble.
 */
export function ThreadAgentWorkingIndicator({
  agents,
  channelId = null,
  className,
  onOpenAgentSession,
  profiles,
  workingBotPubkeys,
}: ThreadAgentWorkingIndicatorProps) {
  const workingAgents = React.useMemo(() => {
    const workingSet = new Set(
      workingBotPubkeys.map((pubkey) => pubkey.toLowerCase()),
    );
    return agents.filter((agent) => workingSet.has(agent.pubkey.toLowerCase()));
  }, [agents, workingBotPubkeys]);

  const statuses = useWorkingAgentStatuses(workingAgents, channelId);
  const [focusIndex, setFocusIndex] = React.useState(0);

  React.useEffect(() => {
    if (statuses.length <= 1) {
      return;
    }
    const interval = window.setInterval(() => {
      setFocusIndex((current) => (current + 1) % statuses.length);
    }, 3200);
    return () => window.clearInterval(interval);
  }, [statuses.length]);

  if (statuses.length === 0) {
    return null;
  }

  const focus = statuses[focusIndex % statuses.length] ?? statuses[0];
  const ariaLabel = formatAgentWorkingStatusLabel(statuses, focusIndex);
  const clickable = Boolean(onOpenAgentSession && focus);
  const headlines = focus.headlines;
  const hasCot = headlines.length > 0;

  const body = (
    <div className="flex min-w-0 w-full items-start gap-2">
      <div className="flex shrink-0 items-center overflow-visible -space-x-1 pt-0.5">
        {statuses.slice(0, 2).map(({ agent }) => (
          <UserAvatar
            avatarUrl={
              profiles?.[agent.pubkey.toLowerCase()]?.avatarUrl ?? null
            }
            className="!h-5 !w-5 border border-background text-3xs"
            displayName={agent.name}
            fallbackDelayMs={0}
            key={agent.pubkey}
            size="xs"
          />
        ))}
      </div>
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        {statuses.length > 1 ? (
          <p className="truncate text-2xs font-medium text-muted-foreground/70">
            {focus.agent.name || "Agent"}
            {statuses.length > 1 ? ` · +${statuses.length - 1}` : null}
          </p>
        ) : null}
        {hasCot ? (
          <ul
            aria-label="Agent activity"
            className="flex min-w-0 flex-col gap-0.5"
            data-testid="thread-agent-cot-stack"
          >
            {headlines.map((line, index) => {
              const isLatest = index === headlines.length - 1;
              // Stable among the small fixed stack; lines can repeat so pair
              // with reverse-rank (not raw index) for uniqueness.
              const rankFromEnd = headlines.length - index;
              return (
                <li
                  className={cn(
                    "min-w-0 truncate text-xs leading-4",
                    isLatest
                      ? "font-medium text-muted-foreground"
                      : "text-muted-foreground/55",
                  )}
                  key={`${rankFromEnd}:${line}`}
                >
                  {isLatest ? <Shimmer>{line}</Shimmer> : line}
                </li>
              );
            })}
          </ul>
        ) : (
          <p
            className="flex min-w-0 items-center gap-1.5 text-xs font-medium text-muted-foreground"
            data-testid="thread-agent-working-label"
          >
            <Shimmer>Thinking…</Shimmer>
          </p>
        )}
        <TurnLivenessIndicator className="mt-0.5 opacity-40" />
      </div>
    </div>
  );

  if (!clickable) {
    return (
      <div
        aria-live="polite"
        className={cn(
          "min-w-0 rounded-lg border border-border/50 bg-muted/30 px-2 py-1.5",
          className,
        )}
        data-testid="thread-agent-working-indicator"
      >
        {body}
      </div>
    );
  }

  return (
    <button
      aria-label={`${ariaLabel}. View activity.`}
      className={cn(
        "min-w-0 w-full rounded-lg border border-border/50 bg-muted/30 px-2 py-1.5 text-left outline-hidden transition-colors hover:border-border hover:bg-muted/50 focus-visible:ring-1 focus-visible:ring-ring",
        className,
      )}
      data-testid="thread-agent-working-indicator"
      onClick={() => {
        if (focus) {
          onOpenAgentSession?.(focus.agent.pubkey, channelId);
        }
      }}
      type="button"
    >
      {body}
    </button>
  );
}
