import * as React from "react";
import {
  Clock3,
  Octagon,
  Settings,
  Sparkles,
  TerminalSquare,
} from "lucide-react";
import { toast } from "sonner";

import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { scopeByChannel } from "@/features/agents/ui/agentSessionPanelLayout";
import type {
  ObserverEvent,
  TranscriptItem,
} from "@/features/agents/ui/agentSessionTypes";
import { ManagedAgentSessionPanel } from "@/features/agents/ui/ManagedAgentSessionPanel";
import {
  useAgentTranscript,
  useObserverEvents,
} from "@/features/agents/ui/useObserverEvents";
import { cancelManagedAgentTurn } from "@/shared/api/agentControl";
import type { Channel } from "@/shared/api/types";
import { useEscapeKey } from "@/shared/hooks/useEscapeKey";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { useStickToBottom } from "@/shared/hooks/useStickToBottom";
import { useNow } from "@/shared/lib/useNow";
import { AuxiliaryPanel } from "@/shared/layout/AuxiliaryPanel";
import { AuxiliaryPanelBody } from "@/shared/layout/AuxiliaryPanel";
import {
  AuxiliaryPanelHeader,
  AuxiliaryPanelHeaderActions,
  AuxiliaryPanelHeaderGroup,
  AuxiliaryPanelHeaderTitleBlock,
} from "@/shared/layout/AuxiliaryPanel";
import { Button } from "@/shared/ui/button";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Switch } from "@/shared/ui/switch";
import {
  setTranscriptAnimationEnabled,
  useTranscriptAnimationEnabled,
} from "@/features/agents/ui/transcriptAnimationPreference";
import {
  setTranscriptTimestampsEnabled,
  useTranscriptTimestampsEnabled,
} from "@/features/agents/ui/transcriptTimestampPreference";
import type { ChannelAgentSessionAgent } from "./useChannelAgentSessions";

type AgentSessionThreadPanelProps = {
  agent: ChannelAgentSessionAgent;
  channel: Channel | null;
  channelId?: string | null;
  canInterruptTurn: boolean;
  isWorking: boolean;
  layout?: "standalone" | "split";
  isSinglePanelView?: boolean;
  profiles?: UserProfileLookup;
  /**
   * Fired by the header back arrow. Restores the pane this panel replaced
   * (thread or profile) via the captured return target — see
   * useChannelAgentSessions.backFromAgentSession. Omit when there is no
   * target (composer/no-pane open, direct/restored URL): the arrow hides
   * and the close affordance is the fallback.
   */
  onBack?: () => void;
  onClose: () => void;
  widthPx: number;
  transparentChrome?: boolean;
};

export function AgentSessionThreadPanel({
  agent,
  canInterruptTurn,
  channel,
  channelId = null,
  isWorking,
  layout = "standalone",
  isSinglePanelView = false,
  profiles,
  onBack,
  onClose,
  widthPx,
  transparentChrome = false,
}: AgentSessionThreadPanelProps) {
  const isLive = isManagedAgentActive(agent);
  const isOverlay = useIsThreadPanelOverlay();
  const canStopCurrentTurn = isWorking && canInterruptTurn;
  useEscapeKey(onClose, isOverlay || isSinglePanelView);

  const { ref: scrollRef, onScroll } = useStickToBottom<HTMLDivElement>();
  const sessionChannelId = channelId ?? channel?.id ?? null;
  const now = useNow(1000);
  const { events } = useObserverEvents(isLive, agent.pubkey);
  const transcript = useAgentTranscript(isLive, agent.pubkey);
  const scopedEvents = React.useMemo(
    () => scopeByChannel(events, sessionChannelId),
    [events, sessionChannelId],
  );
  const scopedTranscript = React.useMemo(
    () => scopeByChannel(transcript, sessionChannelId),
    [sessionChannelId, transcript],
  );
  const latestActivityAt = React.useMemo(
    () =>
      getLatestActivityTimestamp({
        events: scopedEvents,
        transcript: scopedTranscript,
      }),
    [scopedEvents, scopedTranscript],
  );
  const lastUpdatedLabel = formatLastUpdatedLabel(latestActivityAt, now);
  const lastUpdatedTitle =
    latestActivityAt === null
      ? undefined
      : `Last updated ${new Date(latestActivityAt).toLocaleString()}`;
  const rawFeedScopeKey = `${agent.pubkey}:${sessionChannelId ?? "all"}`;
  const [rawFeedState, setRawFeedState] = React.useState(() => ({
    scopeKey: rawFeedScopeKey,
    show: false,
  }));
  const showRawFeed =
    rawFeedState.scopeKey === rawFeedScopeKey && rawFeedState.show;
  const handleRawFeedChange = React.useCallback(
    (checked: boolean) => {
      setRawFeedState({ scopeKey: rawFeedScopeKey, show: checked });
    },
    [rawFeedScopeKey],
  );
  const animateActivity = useTranscriptAnimationEnabled();
  const showTimestamps = useTranscriptTimestampsEnabled();
  async function handleInterruptTurn() {
    if (!channel) {
      return;
    }

    try {
      await cancelManagedAgentTurn(agent.pubkey, channel.id);
      toast.success(
        `Stop signal sent to ${agent.name}. It may take a moment to respond.`,
      );
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : `Failed to stop ${agent.name}'s current turn.`,
      );
    }
  }

  const agentHeaderActions = (
    <AuxiliaryPanelHeaderActions>
      {isLive ? (
        <DropdownMenu modal={false}>
          <DropdownMenuTrigger asChild>
            <Button
              aria-label="Open activity settings"
              className="relative"
              data-testid="agent-session-settings-menu-trigger"
              size="icon"
              title="Activity settings"
              type="button"
              variant="ghost"
            >
              <Settings />
              {canStopCurrentTurn ? (
                <span
                  aria-hidden="true"
                  className="absolute right-1 bottom-1 h-2 w-2 rounded-full bg-primary ring-2 ring-background"
                  data-testid="agent-session-settings-live-badge"
                />
              ) : null}
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align="end"
            className="min-w-56"
            onCloseAutoFocus={(event) => event.preventDefault()}
          >
            <DropdownMenuItem
              className="items-start gap-3"
              data-testid="agent-session-toggle-raw-feed"
              onSelect={(event) => {
                event.preventDefault();
                handleRawFeedChange(!showRawFeed);
              }}
              title={
                showRawFeed
                  ? "Hide raw JSON-RPC payloads."
                  : channel
                    ? "Show raw JSON-RPC payloads for this channel."
                    : "Show raw JSON-RPC payloads for this agent."
              }
            >
              <span className="min-w-0 flex-1">
                <span className="flex items-center gap-2 text-sm font-medium">
                  <TerminalSquare className="h-4 w-4 text-muted-foreground" />
                  Raw
                </span>
                <span className="mt-0.5 block text-xs text-muted-foreground">
                  Show raw JSON-RPC activity.
                </span>
              </span>
              <Switch
                aria-hidden="true"
                checked={showRawFeed}
                className="pointer-events-none mt-0.5"
                tabIndex={-1}
              />
            </DropdownMenuItem>
            <DropdownMenuItem
              className="items-start gap-3"
              data-testid="agent-session-toggle-animate-activity"
              disabled={showRawFeed}
              onSelect={(event) => {
                event.preventDefault();
                setTranscriptAnimationEnabled(!animateActivity);
              }}
              title={
                showRawFeed
                  ? "Raw activity rows don't animate in."
                  : animateActivity
                    ? "Stop animating new activity rows."
                    : "Animate new activity rows as they arrive."
              }
            >
              <span className="min-w-0 flex-1">
                <span className="flex items-center gap-2 text-sm font-medium">
                  <Sparkles className="h-4 w-4 text-muted-foreground" />
                  Show Animations
                </span>
              </span>
              <Switch
                aria-hidden="true"
                checked={animateActivity && !showRawFeed}
                className="pointer-events-none mt-0.5"
                tabIndex={-1}
              />
            </DropdownMenuItem>
            <DropdownMenuItem
              className="items-start gap-3"
              data-testid="agent-session-toggle-show-timestamps"
              onSelect={(event) => {
                event.preventDefault();
                setTranscriptTimestampsEnabled(!showTimestamps);
              }}
              title={
                showTimestamps
                  ? "Hide per-row activity timestamps."
                  : "Show a timestamp under each activity row."
              }
            >
              <span className="min-w-0 flex-1">
                <span className="flex items-center gap-2 text-sm font-medium">
                  <Clock3 className="h-4 w-4 text-muted-foreground" />
                  Show Timestamps
                </span>
              </span>
              <Switch
                aria-hidden="true"
                checked={showTimestamps}
                className="pointer-events-none mt-0.5"
                tabIndex={-1}
              />
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              className="items-start gap-3"
              data-testid="agent-session-stop-turn"
              disabled={!canStopCurrentTurn}
              onSelect={() => {
                void handleInterruptTurn();
              }}
              title={
                canStopCurrentTurn
                  ? "Interrupt the current ACP turn without stopping the agent process."
                  : isWorking
                    ? "Only locally managed agents can be interrupted from this workspace."
                    : "Available while the agent is working."
              }
            >
              <Octagon className="mt-0.5 h-4 w-4 text-muted-foreground" />
              <span className="min-w-0 flex-1">
                <span className="block text-sm font-medium">
                  Stop current turn
                </span>
                {!canStopCurrentTurn ? (
                  <span className="mt-0.5 block text-xs text-muted-foreground">
                    {isWorking
                      ? "Only available for locally managed agents."
                      : "Available while the agent is working."}
                  </span>
                ) : null}
              </span>
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
    </AuxiliaryPanelHeaderActions>
  );

  const agentHeaderContent = (
    <>
      <AuxiliaryPanelHeaderGroup
        align="start"
        backButtonAriaLabel="Back from activity"
        backButtonTestId="agent-session-back"
        onBack={onBack}
      >
        <AuxiliaryPanelHeaderTitleBlock
          subtitle={lastUpdatedLabel}
          subtitleTitle={lastUpdatedTitle}
          title={showRawFeed ? "Raw ACP Activity" : "Activity"}
        />
      </AuxiliaryPanelHeaderGroup>
      {agentHeaderActions}
    </>
  );

  return (
    <AuxiliaryPanel
      isSinglePanelView={isSinglePanelView}
      layout={layout}
      onClose={onClose}
      testId="agent-session-thread-panel"
      transparentChrome={transparentChrome}
      widthPx={widthPx}
      header={
        <AuxiliaryPanelHeader
          backdrop={layout !== "split" && !isOverlay}
          backdropSurface="soft"
          inset={layout !== "split" ? "wide" : "default"}
        >
          {agentHeaderContent}
        </AuxiliaryPanelHeader>
      }
    >
      <AuxiliaryPanelBody
        ref={scrollRef}
        onScroll={onScroll}
        className="overflow-y-auto px-3 pb-4"
        panelPadding
      >
        <ManagedAgentSessionPanel
          agent={agent}
          channelId={sessionChannelId}
          className="border-0 bg-transparent p-0 shadow-none"
          emptyDescription={
            sessionChannelId
              ? `Mention ${agent.name} in the channel to see its work here.`
              : `Mention ${agent.name} in any channel to see its work here.`
          }
          profiles={profiles}
          rawLayout="exclusive"
          showHeader={false}
          showRaw={showRawFeed}
        />
      </AuxiliaryPanelBody>
    </AuxiliaryPanel>
  );
}

function getLatestActivityTimestamp({
  events,
  transcript,
}: {
  events: readonly ObserverEvent[];
  transcript: readonly TranscriptItem[];
}): number | null {
  let latest: number | null = null;

  const record = (timestamp: string) => {
    const parsed = Date.parse(timestamp);
    if (!Number.isFinite(parsed)) {
      return;
    }

    if (latest === null || parsed > latest) {
      latest = parsed;
    }
  };

  for (const event of events) {
    record(event.timestamp);
  }

  for (const item of transcript) {
    record(item.timestamp);
  }

  return latest;
}

function formatLastUpdatedLabel(timestamp: number | null, now: number): string {
  if (timestamp === null) {
    return "No updates yet";
  }

  return `Last updated ${formatRelativeActivityTime(timestamp, now)}`;
}

function formatRelativeActivityTime(timestamp: number, now: number): string {
  const elapsedMs = Math.max(0, now - timestamp);
  const totalSeconds = Math.floor(elapsedMs / 1_000);

  if (totalSeconds < 60) {
    return "just now";
  }

  const totalMinutes = Math.floor(totalSeconds / 60);
  if (totalMinutes < 60) {
    return `${totalMinutes}m ago`;
  }

  const totalHours = Math.floor(totalMinutes / 60);
  if (totalHours < 24) {
    return `${totalHours}h ago`;
  }

  const totalDays = Math.floor(totalHours / 24);
  if (totalDays < 7) {
    return `${totalDays}d ago`;
  }

  const totalWeeks = Math.floor(totalDays / 7);
  return `${totalWeeks}w ago`;
}
