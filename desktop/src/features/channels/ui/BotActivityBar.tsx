import * as React from "react";
import { Loader2 } from "lucide-react";

import { useAgentTranscript } from "@/features/agents/ui/useObserverEvents";
import {
  getActiveTurnCountForChannel,
  subscribeActiveAgentTurns,
} from "@/features/agents/activeAgentTurnsStore";
import {
  getAgentWorkingState,
  subscribeAgentWorkingSignal,
} from "@/features/agents/agentWorkingSignal";
import {
  buildStableActivityStatus,
  formatElapsed,
  formatStatusSegments,
} from "@/features/channels/ui/botActivityStatus";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ManagedAgent } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import {
  DEFAULT_POPOVER_HOVER_OPEN_DELAY_MS,
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/shared/ui/popover";
import { Shimmer } from "@/shared/ui/Shimmer";
import { UserAvatar } from "@/shared/ui/UserAvatar";

export type BotActivityAgent = Pick<ManagedAgent, "pubkey" | "name">;

type BotActivityBarProps = {
  agents: BotActivityAgent[];
  channelId?: string | null;
  /** Thread root id when this bar lives in a thread composer — locks the
   *  status detail onto that thread's turn instead of the channel's newest. */
  threadRootId?: string | null;
  onOpenAgentSession: (pubkey: string, channelId?: string | null) => void;
  openAgentSessionPubkey: string | null;
  profiles?: UserProfileLookup;
  workingBotPubkeys: string[];
  variant?: "toolbar" | "inline";
};

const HOVER_CLOSE_DELAY_MS = 180;
const ELAPSED_TICK_MS = 1000;

export function BotActivityComposerAction({
  agents,
  channelId = null,
  threadRootId = null,
  onOpenAgentSession,
  openAgentSessionPubkey,
  profiles,
  workingBotPubkeys,
  variant = "toolbar",
}: BotActivityBarProps) {
  const [open, setOpen] = React.useState(false);
  const hoverTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );

  const workingAgents = React.useMemo(() => {
    const workingSet = new Set(
      workingBotPubkeys.map((pubkey) => pubkey.toLowerCase()),
    );

    return agents.filter((agent) => workingSet.has(agent.pubkey.toLowerCase()));
  }, [agents, workingBotPubkeys]);
  const singleWorkingAgent =
    workingAgents.length === 1 ? (workingAgents[0] ?? null) : null;
  const transcript = useAgentTranscript(
    Boolean(singleWorkingAgent),
    singleWorkingAgent?.pubkey,
  );
  const activityStatus = React.useMemo(
    () =>
      singleWorkingAgent
        ? buildStableActivityStatus(transcript, channelId, threadRootId)
        : null,
    [channelId, singleWorkingAgent, threadRootId, transcript],
  );

  // Turn-start anchor for the elapsed segment, observer-primary with a
  // typing fallback — the same signal every other working affordance reads.
  const singleWorkingPubkey = singleWorkingAgent?.pubkey ?? null;
  const workingState = React.useSyncExternalStore(
    subscribeAgentWorkingSignal,
    React.useCallback(
      () => getAgentWorkingState(singleWorkingPubkey, channelId),
      [channelId, singleWorkingPubkey],
    ),
  );
  const anchorAt = React.useMemo(() => {
    if (!singleWorkingAgent || workingState.channels.length === 0) {
      return null;
    }
    if (channelId) {
      const scoped = workingState.channels.find(
        (channel) => channel.channelId === channelId,
      );
      if (scoped) {
        return scoped.anchorAt;
      }
    }
    return workingState.channels.reduce(
      (earliest, channel) => Math.min(earliest, channel.anchorAt),
      Number.POSITIVE_INFINITY,
    );
  }, [channelId, singleWorkingAgent, workingState]);

  const channelTurnCount = React.useSyncExternalStore(
    subscribeActiveAgentTurns,
    React.useCallback(
      () => getActiveTurnCountForChannel(singleWorkingPubkey, channelId),
      [channelId, singleWorkingPubkey],
    ),
  );

  // Re-render once a second while a turn is running so the elapsed segment
  // ticks in place — the only part of the line that changes on its own.
  const [, tick] = React.useReducer((count: number) => count + 1, 0);
  React.useEffect(() => {
    if (anchorAt === null) {
      return;
    }

    const interval = window.setInterval(tick, ELAPSED_TICK_MS);

    return () => window.clearInterval(interval);
  }, [anchorAt]);

  const clearHoverTimer = React.useCallback(() => {
    if (hoverTimerRef.current !== null) {
      clearTimeout(hoverTimerRef.current);
      hoverTimerRef.current = null;
    }
  }, []);

  const openWithDelay = React.useCallback(() => {
    clearHoverTimer();
    hoverTimerRef.current = setTimeout(() => {
      setOpen(true);
    }, DEFAULT_POPOVER_HOVER_OPEN_DELAY_MS);
  }, [clearHoverTimer]);

  const closeWithDelay = React.useCallback(() => {
    clearHoverTimer();
    hoverTimerRef.current = setTimeout(() => {
      setOpen(false);
    }, HOVER_CLOSE_DELAY_MS);
  }, [clearHoverTimer]);

  const keepOpen = React.useCallback(() => {
    clearHoverTimer();
  }, [clearHoverTimer]);

  React.useEffect(() => {
    return () => clearHoverTimer();
  }, [clearHoverTimer]);

  if (workingAgents.length === 0) {
    return null;
  }

  const agentAvatarUrl = (agent: BotActivityAgent) =>
    profiles?.[agent.pubkey.toLowerCase()]?.avatarUrl ?? null;
  const selectedPubkey = openAgentSessionPubkey?.toLowerCase() ?? null;
  const triggerLabel =
    workingAgents.length === 1
      ? `${workingAgents[0]?.name ?? "Agent"} is working`
      : `${workingAgents.length} agents working`;
  const isInline = variant === "inline";
  const elapsed =
    anchorAt !== null && Number.isFinite(anchorAt)
      ? formatElapsed(Date.now() - anchorAt)
      : null;
  const visibleStatusLabel =
    workingAgents.length === 1
      ? [
          workingAgents[0]?.name ?? "Agent",
          // Parallel turns in one channel would interleave in a single
          // detailed line, so past one turn the channel bar aggregates;
          // each thread's own bar still carries that thread's detail.
          channelTurnCount > 1 && !threadRootId
            ? [
                `${channelTurnCount} threads`,
                ...(elapsed ? [elapsed] : []),
              ].join(" · ")
            : formatStatusSegments(
                activityStatus ?? {
                  activity: "Working",
                  toolCount: 0,
                  context: null,
                },
                elapsed,
              ),
        ].join(" · ")
      : `${workingAgents[0]?.name ?? "Agent"} +${workingAgents.length - 1}`;

  return (
    <Popover onOpenChange={setOpen} open={open}>
      <PopoverTrigger asChild>
        <button
          aria-label={`${triggerLabel}. View activity.`}
          className={cn(
            "inline-flex items-center justify-center rounded-full border border-border/60 bg-background font-medium text-muted-foreground transition-colors hover:border-primary/30 hover:bg-primary/5 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:border-primary/40 data-[state=open]:bg-primary/10 data-[state=open]:text-primary",
            isInline
              ? "min-w-0 gap-1.5 overflow-visible border-transparent bg-transparent px-0 text-xs font-normal leading-normal shadow-none hover:border-transparent hover:bg-transparent data-[state=open]:border-transparent data-[state=open]:bg-transparent"
              : "h-9 min-w-9 gap-1.5 px-2 text-xs",
          )}
          data-testid="bot-activity-composer-trigger"
          onBlur={closeWithDelay}
          onClick={() => {
            clearHoverTimer();
            setOpen((current) => !current);
          }}
          onFocus={() => setOpen(true)}
          onMouseEnter={openWithDelay}
          onMouseLeave={closeWithDelay}
          type="button"
        >
          <span className="flex h-4.5 items-center overflow-visible -space-x-1">
            {workingAgents.slice(0, 2).map((agent) => (
              <UserAvatar
                avatarUrl={agentAvatarUrl(agent)}
                className={cn(
                  "border border-background",
                  isInline ? "!h-4.5 !w-4.5 text-3xs" : "shrink-0",
                )}
                displayName={agent.name}
                shape="squircle"
                fallbackDelayMs={isInline ? 0 : undefined}
                key={agent.pubkey}
                size="xs"
              />
            ))}
          </span>
          {workingAgents.length > 2 ? (
            <span className="text-2xs leading-none">
              +{workingAgents.length - 2}
            </span>
          ) : null}
          <span
            className={cn(
              isInline
                ? "flex h-4.5 min-w-0 flex-1 items-center overflow-visible leading-none"
                : "sr-only",
            )}
          >
            {isInline ? (
              <Shimmer className="-my-px truncate py-px">
                {visibleStatusLabel}
              </Shimmer>
            ) : (
              "working"
            )}
          </span>
          {isInline ? null : (
            <Loader2 className="h-4 w-4 shrink-0 animate-spin opacity-70" />
          )}
        </button>
      </PopoverTrigger>
      <PopoverContent
        align={isInline ? "start" : "end"}
        className="w-64 p-1"
        onMouseEnter={keepOpen}
        onMouseLeave={closeWithDelay}
        onOpenAutoFocus={(event) => event.preventDefault()}
        side="top"
        sideOffset={8}
      >
        <div className="px-2 py-1 text-xs font-medium text-muted-foreground">
          Agents working
        </div>
        <div className="mt-1 flex flex-col gap-1">
          {workingAgents.map((agent) => {
            const isSelected = selectedPubkey === agent.pubkey.toLowerCase();

            return (
              <button
                className={cn(
                  "flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-sm transition-colors",
                  isSelected
                    ? "bg-primary/10 text-primary"
                    : "text-foreground hover:bg-accent hover:text-accent-foreground",
                )}
                data-testid={`bot-activity-composer-item-${agent.pubkey}`}
                key={agent.pubkey}
                onClick={() => {
                  clearHoverTimer();
                  setOpen(false);
                  onOpenAgentSession(agent.pubkey, channelId);
                }}
                type="button"
              >
                <UserAvatar
                  avatarUrl={agentAvatarUrl(agent)}
                  className="shrink-0"
                  displayName={agent.name}
                  shape="squircle"
                  size="sm"
                />
                <span className="min-w-0 flex-1 truncate">{agent.name}</span>
                <span className="shrink-0 whitespace-nowrap text-xs font-medium opacity-80">
                  View activity
                </span>
                <Loader2 className="h-4 w-4 shrink-0 animate-spin text-muted-foreground/70" />
              </button>
            );
          })}
        </div>
      </PopoverContent>
    </Popover>
  );
}
