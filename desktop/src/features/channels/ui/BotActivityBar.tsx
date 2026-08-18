import * as React from "react";
import { AlertTriangle, Loader2 } from "lucide-react";

import { useOpenCircuitAgents } from "@/features/agents/agentCircuitHooks";
import { useAgentTranscript } from "@/features/agents/ui/useObserverEvents";
import {
  getActivityHeadline,
  isMeaningfulItem,
  isSpineItem,
} from "@/features/agents/ui/agentSessionTranscriptPresentation";
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
  onOpenAgentSession: (pubkey: string, channelId?: string | null) => void;
  openAgentSessionPubkey: string | null;
  profiles?: UserProfileLookup;
  workingBotPubkeys: string[];
  variant?: "toolbar" | "inline";
};

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
  // A suspended (crashed, circuit-open) agent is never "working" — this is
  // the in-channel counterpart to the persistent circuit badge on the Agents
  // screen, so a user typing to an agent that just crashed sees it here too
  // instead of only after navigating to Settings.
  const suspendedAgents = useOpenCircuitAgents(agents);
  const hasSuspended = suspendedAgents.length > 0;
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

  React.useEffect(() => {
    if (activityHeadlines.length <= 1) {
      return;
    }

    const interval = window.setInterval(() => {
      setHeadlineIndex((current) => (current + 1) % activityHeadlines.length);
    }, HEADLINE_ROTATION_MS);

    return () => window.clearInterval(interval);
  }, [activityHeadlines.length]);

  if (workingAgents.length === 0 && !hasSuspended) {
    return null;
  }

  const agentAvatarUrl = (agent: BotActivityAgent) =>
    profiles?.[agent.pubkey.toLowerCase()]?.avatarUrl ?? null;
  const selectedPubkey = openAgentSessionPubkey?.toLowerCase() ?? null;
  // Suspended takes priority in the trigger — an agent that's dead is more
  // urgent to notice than one that's merely busy.
  const headlineAgents = hasSuspended ? suspendedAgents : workingAgents;
  const triggerLabel = hasSuspended
    ? suspendedAgents.length === 1
      ? `${suspendedAgents[0]?.name ?? "Agent"} suspended — repeated crashes`
      : `${suspendedAgents.length} agents suspended`
    : workingAgents.length === 1
      ? `${workingAgents[0]?.name ?? "Agent"} is working`
      : `${workingAgents.length} agents working`;
  const isInline = variant === "inline";
  const visibleStatusLabel = hasSuspended
    ? suspendedAgents.length === 1
      ? `${suspendedAgents[0]?.name ?? "Agent"}: Suspended`
      : `${suspendedAgents[0]?.name ?? "Agent"} +${suspendedAgents.length - 1} suspended`
    : workingAgents.length === 1
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
            hasSuspended &&
              !isInline &&
              "border-destructive/40 text-destructive hover:border-destructive/60 hover:bg-destructive/5 hover:text-destructive data-[state=open]:border-destructive/60 data-[state=open]:bg-destructive/10 data-[state=open]:text-destructive",
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
            {headlineAgents.slice(0, 2).map((agent) => (
              <UserAvatar
                avatarUrl={agentAvatarUrl(agent)}
                className={cn(
                  "border border-background",
                  isInline ? "!h-4.5 !w-4.5 text-3xs" : "shrink-0",
                )}
                displayName={agent.name}
                fallbackDelayMs={isInline ? 0 : undefined}
                key={agent.pubkey}
                size="xs"
              />
            ))}
          </span>
          {headlineAgents.length > 2 ? (
            <span className="text-2xs leading-none">
              +{headlineAgents.length - 2}
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
              <Shimmer
                className={cn(
                  "-my-px truncate py-px",
                  hasSuspended && "text-destructive",
                )}
              >
                {visibleStatusLabel}
              </Shimmer>
            ) : hasSuspended ? (
              "suspended"
            ) : (
              "working"
            )}
          </span>
          {isInline ? null : hasSuspended ? (
            <AlertTriangle className="h-4 w-4 shrink-0 opacity-70" />
          ) : (
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
        {hasSuspended ? (
          <AgentPopoverSection
            agentAvatarUrl={agentAvatarUrl}
            agents={suspendedAgents}
            heading="Agents suspended"
            headingClassName="text-destructive"
            itemStatusLabel="Suspended"
            onCloseWithSelection={(pubkey) => {
              clearHoverTimer();
              setOpen(false);
              onOpenAgentSession(pubkey, channelId);
            }}
            selectedPubkey={selectedPubkey}
            trailingIcon={
              <AlertTriangle className="h-4 w-4 shrink-0 text-destructive/70" />
            }
            variant="destructive"
          />
        ) : null}
        {workingAgents.length > 0 ? (
          <AgentPopoverSection
            agentAvatarUrl={agentAvatarUrl}
            agents={workingAgents}
            heading="Agents working"
            headingClassName={cn(
              "text-muted-foreground",
              hasSuspended && "mt-2",
            )}
            itemStatusLabel="View activity"
            onCloseWithSelection={(pubkey) => {
              clearHoverTimer();
              setOpen(false);
              onOpenAgentSession(pubkey, channelId);
            }}
            selectedPubkey={selectedPubkey}
            trailingIcon={
              <Loader2 className="h-4 w-4 shrink-0 animate-spin text-muted-foreground/70" />
            }
            variant="default"
          />
        ) : null}
      </PopoverContent>
    </Popover>
  );
}

/** One labeled group of agent rows in the popover — shared shape for the
 * working and suspended sections, which differ only in styling/copy/icon. */
function AgentPopoverSection({
  agentAvatarUrl,
  agents,
  heading,
  headingClassName,
  itemStatusLabel,
  onCloseWithSelection,
  selectedPubkey,
  trailingIcon,
  variant,
}: {
  agentAvatarUrl: (agent: BotActivityAgent) => string | null;
  agents: BotActivityAgent[];
  heading: string;
  headingClassName?: string;
  itemStatusLabel: string;
  onCloseWithSelection: (pubkey: string) => void;
  selectedPubkey: string | null;
  trailingIcon: React.ReactNode;
  variant: "default" | "destructive";
}) {
  return (
    <>
      <div className={cn("px-2 py-1 text-xs font-medium", headingClassName)}>
        {heading}
      </div>
      <div className="mt-1 flex flex-col gap-1">
        {agents.map((agent) => {
          const isSelected = selectedPubkey === agent.pubkey.toLowerCase();

          return (
            <button
              className={cn(
                "flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-sm transition-colors",
                variant === "destructive"
                  ? isSelected
                    ? "bg-destructive/10 text-destructive"
                    : "text-foreground hover:bg-destructive/5 hover:text-destructive"
                  : isSelected
                    ? "bg-primary/10 text-primary"
                    : "text-foreground hover:bg-accent hover:text-accent-foreground",
              )}
              data-testid={`bot-activity-composer-item-${agent.pubkey}`}
              key={agent.pubkey}
              onClick={() => onCloseWithSelection(agent.pubkey)}
              type="button"
            >
              <UserAvatar
                avatarUrl={agentAvatarUrl(agent)}
                className="shrink-0"
                displayName={agent.name}
                size="sm"
              />
              <span className="min-w-0 flex-1 truncate">{agent.name}</span>
              <span className="shrink-0 whitespace-nowrap text-xs font-medium opacity-80">
                {itemStatusLabel}
              </span>
              {trailingIcon}
            </button>
          );
        })}
      </div>
    </>
  );
}
