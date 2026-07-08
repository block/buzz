import * as React from "react";
import { Loader2 } from "lucide-react";

import { useAgentTranscript } from "@/features/agents/ui/useObserverEvents";
import {
  getActivityHeadline,
  isMeaningfulItem,
  isSpineItem,
} from "@/features/agents/ui/agentSessionTranscriptPresentation";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ManagedAgent } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Shimmer } from "@/shared/ui/Shimmer";
import { UserAvatar } from "@/shared/ui/UserAvatar";

export type BotActivityAgent = Pick<ManagedAgent, "pubkey" | "name">;

type BotActivityBarProps = {
  agents: BotActivityAgent[];
  channelId?: string | null;
  onOpenAgentSession: (pubkey: string, channelId?: string | null) => void;
  openAgentSessionPubkey: string | null;
  profiles?: UserProfileLookup;
  workingBotPubkeys: string[];
  variant?: "toolbar" | "inline";
};

const HOVER_OPEN_DELAY_MS = 150;
const HOVER_CLOSE_DELAY_MS = 180;
const HEADLINE_ROTATION_MS = 2200;

export function BotActivityComposerAction({
  agents,
  channelId = null,
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
  const activityHeadlines = React.useMemo(() => {
    if (!singleWorkingAgent) {
      return [];
    }

    const seen = new Set<string>();
    const headlines: string[] = [];
    const scopedTranscript = channelId
      ? transcript.filter((item) => item.channelId === channelId)
      : transcript;

    // Two-tier scan: spine items first (reads recede when real work is present).
    // If no spine headlines are found (session start / idle), fall back to all
    // meaningful items so the bar isn't left empty.
    const passFilter: (item: (typeof scopedTranscript)[number]) => boolean =
      scopedTranscript.some(isSpineItem) ? isSpineItem : isMeaningfulItem;

    for (let i = scopedTranscript.length - 1; i >= 0; i--) {
      const item = scopedTranscript[i];
      if (!passFilter(item)) {
        continue;
      }
      const headline = getActivityHeadline(item);
      if (!headline || seen.has(headline)) {
        continue;
      }

      seen.add(headline);
      headlines.unshift(headline);
      if (headlines.length >= 5) {
        break;
      }
    }

    return headlines;
  }, [channelId, singleWorkingAgent, transcript]);
  const [headlineIndex, setHeadlineIndex] = React.useState(0);

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
    }, HOVER_OPEN_DELAY_MS);
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

  React.useEffect(() => {
    if (activityHeadlines.length <= 1) {
      return;
    }

    const interval = window.setInterval(() => {
      setHeadlineIndex((current) => (current + 1) % activityHeadlines.length);
    }, HEADLINE_ROTATION_MS);

    return () => window.clearInterval(interval);
  }, [activityHeadlines.length]);

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
  const visibleStatusLabel =
    workingAgents.length === 1
      ? `${workingAgents[0]?.name ?? "Agent"}: ${
          activityHeadlines[headlineIndex % activityHeadlines.length] ??
          "Working"
        }`
      : `${workingAgents[0]?.name ?? "Agent"} +${workingAgents.length - 1}`;

  return (
    <Popover onOpenChange={setOpen} open={open}>
      <PopoverTrigger asChild>
        <button
          aria-label={`${triggerLabel}. View activity.`}
          className={cn(
            "inline-flex items-center justify-center rounded-full border border-border/60 bg-background font-medium text-muted-foreground transition-colors hover:border-primary/30 hover:bg-primary/5 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:border-primary/40 data-[state=open]:bg-primary/10 data-[state=open]:text-primary",
            isInline
              ? "h-7 min-w-0 gap-2 overflow-visible border-transparent bg-transparent px-0 text-xs font-semibold leading-none shadow-none hover:border-transparent hover:bg-transparent data-[state=open]:border-transparent data-[state=open]:bg-transparent"
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
          <span className="flex items-center overflow-visible py-px -space-x-1">
            {workingAgents.slice(0, 2).map((agent) => (
              <UserAvatar
                avatarUrl={agentAvatarUrl(agent)}
                className={cn(
                  "border border-background",
                  isInline
                    ? "!h-[18px] !w-[18px] shadow-xs ring-1 ring-primary/25 text-3xs"
                    : "shrink-0",
                )}
                displayName={agent.name}
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
              isInline ? "min-w-0 flex-1 overflow-hidden" : "sr-only",
            )}
          >
            {isInline ? (
              <Shimmer className="block truncate">{visibleStatusLabel}</Shimmer>
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
                  size="sm"
                />
                <span className="min-w-0 flex-1 truncate">{agent.name}</span>
                <Loader2 className="h-4 w-4 shrink-0 animate-spin text-muted-foreground/70" />
              </button>
            );
          })}
        </div>
      </PopoverContent>
    </Popover>
  );
}
