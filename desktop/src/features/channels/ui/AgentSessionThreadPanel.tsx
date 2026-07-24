import * as React from "react";
import {
  Clock3,
  Octagon,
  Settings,
  Sparkles,
  TerminalSquare,
} from "lucide-react";
import { toast } from "sonner";

import { useAgentWorking } from "@/features/agents/agentWorkingSignal";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import {
  mergeObserverEventWindows,
  observerEventScrollId,
  scopeByChannel,
} from "@/features/agents/ui/agentSessionPanelLayout";
import { deriveTranscriptBlockIds } from "@/features/agents/ui/agentSessionTranscriptGrouping";
import type { ObserverEvent } from "@/features/agents/ui/agentSessionTypes";
import { ManagedAgentSessionPanel } from "@/features/agents/ui/ManagedAgentSessionPanel";
import { useMeshInferenceLocation } from "@/features/mesh-compute/hooks/useMeshInferenceLocation";
import {
  useArchivedChannelEvents,
  useObserverEvents,
} from "@/features/agents/ui/useObserverEvents";
import { useAnchoredScroll } from "@/features/messages/ui/useAnchoredScroll";
import { useStableArrayShallow } from "@/shared/hooks/useStableReference";
import { cancelManagedAgentTurn } from "@/shared/api/agentControl";
import type { Channel } from "@/shared/api/types";
import { useEscapeKey } from "@/shared/hooks/useEscapeKey";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
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
import { useLoadArchivedObserverEvents } from "@/features/agents/ui/useObserverEvents";
import { useLoadOlderOnScroll } from "@/features/messages/ui/useLoadOlderOnScroll";
import type { ChannelAgentSessionAgent } from "./useChannelAgentSessions";
import { useChannelsQuery } from "@/features/channels/hooks";

type AgentSessionThreadPanelProps = {
  agent: ChannelAgentSessionAgent;
  channel: Channel | null;
  channelId?: string | null;
  canInterruptTurn: boolean;
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
  const sessionChannelId = channelId ?? channel?.id ?? null;
  // Unified working signal, scoped to this panel's channel (or all channels
  // when the panel is unscoped) — observer turns primary, typing fallback.
  const { working: isWorking } = useAgentWorking(
    agent.pubkey,
    sessionChannelId,
  );
  const canStopCurrentTurn = isWorking && canInterruptTurn;
  useEscapeKey(onClose, isOverlay || isSinglePanelView);

  const scrollRef = React.useRef<HTMLDivElement>(null);
  const contentRef = React.useRef<HTMLDivElement>(null);
  const topSentinelRef = React.useRef<HTMLDivElement>(null);
  const now = useNow(1000);
  const { connectionState, events } = useObserverEvents(isLive, agent.pubkey);
  const scopedEvents = React.useMemo(
    () => scopeByChannel(events, sessionChannelId),
    [events, sessionChannelId],
  );
  // Archived channel events merged with live scoped events so the header's
  // "Last updated" timestamp reflects the full loaded history, not just the
  // capped live window. Mirrors ManagedAgentSessionPanel's combinedEvents.
  const archivedChannelEvents = useArchivedChannelEvents(
    agent.pubkey,
    sessionChannelId,
  );
  const combinedHeaderEvents = React.useMemo(
    () => mergeObserverEventWindows(scopedEvents, archivedChannelEvents),
    [scopedEvents, archivedChannelEvents],
  );
  const latestActivityAt = React.useMemo(
    () => getLatestActivityTimestamp(combinedHeaderEvents),
    [combinedHeaderEvents],
  );
  const lastUpdatedLabel = formatLastUpdatedLabel(latestActivityAt, now);
  const lastUpdatedTitle =
    latestActivityAt === null
      ? undefined
      : `Last updated ${new Date(latestActivityAt).toLocaleString()}`;

  const { fetchOlderArchived, hasOlderArchived } =
    useLoadArchivedObserverEvents(
      // Archive history must load regardless of live status — an idle agent's
      // channel should still show its archived observer history. Enable whenever
      // there is a resolved sessionChannelId (the hook's owner_p guard handles
      // the case where no save subscription exists).
      Boolean(sessionChannelId),
      sessionChannelId ?? null,
    );

  useLoadOlderOnScroll({
    fetchOlder: fetchOlderArchived,
    hasOlderMessages: hasOlderArchived,
    isLoading: false,
    scrollContainerRef: scrollRef,
    sentinelRef: topSentinelRef,
  });
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

  // --- Transcript block ids for default Activity mode ---
  // Derive the same display-block keys the inner AgentSessionTranscriptList
  // renders as `data-message-id` so useAnchoredScroll anchors on real DOM rows
  // instead of raw event ids (which live in a disjoint namespace and cause
  // per-event floor writes → flicker + jump-to-tail).
  //
  // latestLiveSessionId is omitted: it only affects boundary `labelState`,
  // never keys (agentSessionTranscriptGrouping.ts:557-574), so we avoid
  // subscribing to the observer store from the outer panel.
  const transcriptBlockIds = React.useMemo(
    () => (showRawFeed ? [] : deriveTranscriptBlockIds(combinedHeaderEvents)),
    [combinedHeaderEvents, showRawFeed],
  );

  // Stabilize the id array by VALUE so the hook's restoration effect (keyed on
  // the `messages` reference) does not fire on every raw event when the block
  // id sequence is unchanged. useStableArrayShallow shallow-compares with
  // Object.is on each string element.
  const stableTranscriptBlockIds = useStableArrayShallow(transcriptBlockIds);

  // Map to {id} objects only when the stabilized array reference changes.
  const transcriptScrollMessages = React.useMemo(
    () => stableTranscriptBlockIds.map((id) => ({ id })),
    [stableTranscriptBlockIds],
  );

  // Raw-mode ids: keyed on (seq, timestamp) — matches RawEventRail's
  // data-message-id. seq resets on agent restart so bare seq can collide;
  // observerEventScrollId disambiguates.
  const rawScrollMessages = React.useMemo(
    () =>
      combinedHeaderEvents.map((event) => ({
        id: observerEventScrollId(event),
      })),
    [combinedHeaderEvents],
  );

  const { onScroll } = useAnchoredScroll({
    // Fold view mode into the reset key so toggling raw ↔ transcript
    // re-initializes the anchor (clean re-pin) instead of carrying an anchor
    // across disjoint id namespaces.
    channelId: `${rawFeedScopeKey}:${showRawFeed ? "raw" : "transcript"}`,
    contentRef,
    isLoading: connectionState === "connecting",
    messages: showRawFeed ? rawScrollMessages : transcriptScrollMessages,
    scrollContainerRef: scrollRef,
  });
  // Scope label input: prefer the passed channel's name; when the pane is
  // channel-scoped without a full Channel object (#1380's channelId prop),
  // resolve the name from the channels cache.
  const channelsQuery = useChannelsQuery({
    enabled: Boolean(sessionChannelId),
  });
  const scopeChannelName = React.useMemo(() => {
    if (!sessionChannelId) {
      return null;
    }
    if (channel && channel.id === sessionChannelId) {
      return channel.name;
    }
    return (
      channelsQuery.data?.find((entry) => entry.id === sessionChannelId)
        ?.name ?? null
    );
  }, [channel, channelsQuery.data, sessionChannelId]);
  const scopeLabel = sessionChannelId
    ? scopeChannelName
      ? `#${scopeChannelName}`
      : "1 channel"
    : "All channels";
  const animateActivity = useTranscriptAnimationEnabled();
  const showTimestamps = useTranscriptTimestampsEnabled();
  // Non-null only for relay-mesh (Buzz shared compute) agents: tells the user
  // where this agent's inference actually runs, e.g. "Qwen3 8B · 3 nodes".
  const meshInferenceLocation = useMeshInferenceLocation(agent.pubkey);
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
                    ? "Only locally managed agents can be interrupted from this community."
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
        {/* Scope label: makes channel-targeted vs all-channels state obvious
            (an all-channels pane can look "wrong" without it). */}
        <span
          className="min-w-0 shrink truncate text-xs text-muted-foreground"
          data-testid="agent-session-scope-label"
        >
          {scopeLabel}
        </span>
        {meshInferenceLocation ? (
          // Where inference runs: only for Buzz shared compute agents, and
          // only once availability is known — never a guess.
          <span
            className="min-w-0 shrink truncate text-xs text-muted-foreground"
            data-testid="agent-session-mesh-inference-location"
            title={meshInferenceLocation.title}
          >
            {meshInferenceLocation.label}
          </span>
        ) : null}
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
        <div ref={topSentinelRef} aria-hidden className="h-px" />
        <div ref={contentRef}>
          <ManagedAgentSessionPanel
            agent={agent}
            channelId={sessionChannelId}
            className="border-0 bg-transparent px-0 py-2 shadow-none"
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
        </div>
      </AuxiliaryPanelBody>
    </AuxiliaryPanel>
  );
}

function getLatestActivityTimestamp(
  events: readonly ObserverEvent[],
): number | null {
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
