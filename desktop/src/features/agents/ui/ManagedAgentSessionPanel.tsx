import * as React from "react";
import {
  CircleAlert,
  CircleDot,
  Clock3,
  TerminalSquare,
  XCircle,
} from "lucide-react";

import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ManagedAgent } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Badge } from "@/shared/ui/badge";
import { Skeleton } from "@/shared/ui/skeleton";
import { Spinner } from "@/shared/ui/spinner";
import {
  AgentSessionTranscriptList,
  type AgentSessionTranscriptEmptyState,
} from "./AgentSessionTranscriptList";
import { RawEventRail } from "./RawEventRail";
import type {
  ConnectionState,
  ObserverEvent,
  TranscriptItem,
} from "./agentSessionTypes";
import type { AgentSessionTranscriptVariant } from "./agentSessionTranscriptContext";
import {
  deriveLatestSessionId,
  mergeObserverEventWindows,
  resolveDisplayEvents,
  resolveRawRailLayout,
  isInjectedTranscriptId,
  mergeTranscriptItems,
  scopeByChannel,
} from "./agentSessionPanelLayout";
import { scopeItemsToThread } from "./threadTurnScope";
import { shorten } from "./agentSessionUtils";
import {
  useObserverEvents,
  useArchivedChannelEvents,
} from "./useObserverEvents";
import { buildTranscriptState } from "./agentSessionTranscript";

type ManagedAgentSessionPanelProps = {
  agent: Pick<ManagedAgent, "pubkey" | "name" | "status"> & {
    avatarUrl?: string | null;
  };
  autoTail?: boolean;
  channelId?: string | null;
  className?: string;
  emptyDescription?: string;
  emptyState?: AgentSessionTranscriptEmptyState;
  panelPadding?: boolean;
  rawLayout?: "responsive" | "exclusive";
  showHeader?: boolean;
  showRaw?: boolean;
  transcriptContentClassName?: string;
  transcriptVariant?: AgentSessionTranscriptVariant;
  profiles?: UserProfileLookup;
  rawEventsOverride?: ObserverEvent[];
  transcriptOverride?: TranscriptItem[];
  /**
   * Transcript rows to interleave with the observer-derived ones, by timestamp.
   * Harness mode uses this to fold in the agent's published replies, which the
   * ACP stream never carries (a reply is sent via the `buzz` CLI, so only the
   * tool call appears — not the answer text).
   */
  extraTranscriptItems?: TranscriptItem[];
  /**
   * Hide only the *trailing* ACP narration row, plus usage rows.
   *
   * Two different assistant texts reach this panel. Intermediate narration —
   * "now let me check X" between tool calls — is exactly the live progress a
   * human wants to steer against, so it stays. The final narration is a
   * third-person self-report ("Answered both questions…") that duplicates the
   * published reply, so it goes. Hiding *all* narration (the earlier behaviour)
   * threw away the useful half.
   *
   * Usage rows are dropped too: the harness surfaces live token counts in its
   * status strip instead of as transcript entries.
   */
  hideTrailingNarration?: boolean;
  /** Inline content beside the turn-liveness indicator. */
  liveStatusSlot?: React.ReactNode;
  /**
   * Event ids of the messages belonging to one thread. When provided, the
   * transcript is scoped to the turns those messages started.
   *
   * This is real scoping, not a time window: every item carries `turnId`, and a
   * user row carries the `messageId` that triggered its turn, so a thread's
   * turns can be identified exactly. A time-based boundary cannot separate two
   * threads in the same channel — the older thread's window necessarily
   * contains the newer thread's frames, which is precisely the leak this
   * replaces.
   */
  threadMessageIds?: ReadonlySet<string>;
  /**
   * Observe the transcript actually rendered. Harness mode derives its live
   * status strip (running commands, tokens, elapsed) from this rather than
   * re-deriving the event pipeline itself.
   */
  onTranscriptChange?: (items: TranscriptItem[]) => void;
};

export function ManagedAgentSessionPanel({
  agent,
  autoTail = false,
  channelId = null,
  className,
  emptyDescription = "Mention this agent in a channel to watch the next turn.",
  emptyState = "idle",
  panelPadding = true,
  rawLayout = "responsive",
  showHeader = true,
  showRaw = true,
  transcriptContentClassName,
  transcriptVariant = "default",
  profiles,
  rawEventsOverride,
  transcriptOverride,
  extraTranscriptItems,
  hideTrailingNarration = false,
  liveStatusSlot,
  threadMessageIds,
  onTranscriptChange,
}: ManagedAgentSessionPanelProps) {
  const hasObserver = isManagedAgentActive(agent);
  // Always read from the store — archived frames are ingested regardless of
  // live status and must be renderable for idle agents with channel history.
  // The `hasObserver` flag still gates the relay subscription (via the
  // useEffect in useObserverEvents) and the empty-state message below.
  const { connectionState, errorMessage, events } = useObserverEvents(
    hasObserver,
    agent.pubkey,
  );

  // Channel-scoped live events (capped at MAX_OBSERVER_EVENTS) and uncapped
  // archived events from SQLite paging. Both are raw ObserverEvent[] — we merge
  // them at the raw-event level and derive a single TranscriptState, so stateful
  // aggregates (tool start/update, plan replacement, permission request/response)
  // are never split across two independent state machines.
  const archivedChannelEvents = useArchivedChannelEvents(
    agent.pubkey,
    channelId,
  );

  const scopedLiveEvents = React.useMemo(
    () => scopeByChannel(events, channelId),
    [channelId, events],
  );

  // Combined raw window: live (scoped) + archive merged by (seq, timestamp),
  // sorted ascending. Used as the single source for both the transcript and the
  // raw event rail / header count.
  const combinedEvents = React.useMemo(
    () => mergeObserverEventWindows(scopedLiveEvents, archivedChannelEvents),
    [scopedLiveEvents, archivedChannelEvents],
  );

  // Derive transcript once from the combined raw window. When transcriptOverride
  // is set (e.g. E2E snapshot specs), bypass both — the caller supplies the full
  // transcript directly.
  const derivedTranscript = React.useMemo(
    () => buildTranscriptState(combinedEvents).items,
    [combinedEvents],
  );
  const displayTranscript = React.useMemo(() => {
    const merged = mergeTranscriptItems(
      transcriptOverride ?? derivedTranscript,
      extraTranscriptItems ?? [],
    );
    // Thread scoping: drop only turns provably belonging to another thread, so
    // the in-flight turn (whose prompt row may not exist yet) stays visible.
    const scoped = scopeItemsToThread(
      merged,
      threadMessageIds,
      isInjectedTranscriptId,
    );

    if (!hideTrailingNarration) {
      return scoped;
    }
    // Usage rows always go — the status strip owns token reporting.
    const withoutUsage = scoped.filter((item) => !item.id.startsWith("usage:"));
    // Drop only the last `assistant:` row, and only once a published reply
    // exists to supersede it. Everything earlier is intermediate progress.
    const hasPublishedReply = withoutUsage.some((item) =>
      item.id.startsWith("reply:"),
    );
    if (!hasPublishedReply) {
      return withoutUsage;
    }
    let lastNarrationIndex = -1;
    withoutUsage.forEach((item, index) => {
      if (item.id.startsWith("assistant:")) {
        lastNarrationIndex = index;
      }
    });
    return lastNarrationIndex === -1
      ? withoutUsage
      : withoutUsage.filter((_, index) => index !== lastNarrationIndex);
  }, [
    derivedTranscript,
    extraTranscriptItems,
    hideTrailingNarration,
    threadMessageIds,
    transcriptOverride,
  ]);

  React.useEffect(() => {
    onTranscriptChange?.(displayTranscript);
  }, [displayTranscript, onTranscriptChange]);

  const displayEvents = React.useMemo(
    () => resolveDisplayEvents(combinedEvents, rawEventsOverride),
    [rawEventsOverride, combinedEvents],
  );

  const latestSessionId = React.useMemo(
    () => deriveLatestSessionId(displayEvents),
    [displayEvents],
  );

  return (
    <section
      className={cn(
        "rounded-lg border border-border/70 bg-background/80 shadow-xs",
        panelPadding && "p-4",
        autoTail && "flex flex-col overflow-hidden",
        className,
      )}
    >
      {showHeader ? (
        <SessionHeader
          connectionState={connectionState}
          eventCount={displayEvents.length}
          hasObserver={hasObserver}
          latestSessionId={latestSessionId}
        />
      ) : null}

      <SessionBody
        agentAvatarUrl={agent.avatarUrl ?? null}
        agentName={agent.name}
        agentPubkey={agent.pubkey}
        connectionState={connectionState}
        autoTail={autoTail}
        channelId={channelId}
        emptyDescription={emptyDescription}
        emptyState={emptyState}
        errorMessage={errorMessage}
        events={displayEvents}
        hasObserver={hasObserver}
        hasTranscriptOverride={transcriptOverride != null}
        profiles={profiles}
        rawLayout={rawLayout}
        showRaw={showRaw}
        liveStatusSlot={liveStatusSlot}
        transcript={displayTranscript}
        transcriptContentClassName={transcriptContentClassName}
        transcriptVariant={transcriptVariant}
      />
    </section>
  );
}

function SessionHeader({
  connectionState,
  eventCount,
  hasObserver,
  latestSessionId,
}: {
  connectionState: ConnectionState;
  eventCount: number;
  hasObserver: boolean;
  latestSessionId: string | null | undefined;
}) {
  return (
    <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <h3 className="text-sm font-semibold tracking-tight">
            Live ACP session
          </h3>
          <ObserverStatusBadge state={connectionState} />
        </div>
        <p className="mt-1 text-sm text-muted-foreground">
          {hasObserver
            ? latestSessionId
              ? `Session ${shorten(latestSessionId)}`
              : "Waiting for the next agent turn."
            : "Restart this local agent to attach the observer feed."}
        </p>
      </div>
      <Badge className="w-fit font-mono" variant="outline">
        {eventCount} event{eventCount === 1 ? "" : "s"}
      </Badge>
    </div>
  );
}

function SessionBody({
  liveStatusSlot,
  agentAvatarUrl,
  agentName,
  agentPubkey,
  autoTail,
  connectionState,
  channelId,
  emptyDescription,
  emptyState,
  errorMessage,
  events,
  hasObserver,
  hasTranscriptOverride,
  profiles,
  rawLayout,
  showRaw,
  transcript,
  transcriptContentClassName,
  transcriptVariant,
}: {
  agentAvatarUrl: string | null;
  agentName: string;
  agentPubkey: string;
  autoTail: boolean;
  channelId: string | null;
  connectionState: ConnectionState;
  emptyDescription: string;
  emptyState: AgentSessionTranscriptEmptyState;
  errorMessage: string | null;
  events: ObserverEvent[];
  hasObserver: boolean;
  hasTranscriptOverride: boolean;
  profiles?: UserProfileLookup;
  rawLayout: "responsive" | "exclusive";
  showRaw: boolean;
  liveStatusSlot?: React.ReactNode;
  transcript: TranscriptItem[];
  transcriptContentClassName?: string;
  transcriptVariant: AgentSessionTranscriptVariant;
}) {
  const rawRail = resolveRawRailLayout(showRaw, rawLayout);

  if (rawRail.mode === "exclusive") {
    return (
      <>
        <RawEventRail events={events} />

        {errorMessage ? (
          <p className="mt-4 inline-flex items-center gap-2 rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
            <CircleAlert className="h-4 w-4" />
            {errorMessage}
          </p>
        ) : null}
      </>
    );
  }

  return (
    <>
      {!hasObserver &&
      !hasTranscriptOverride &&
      transcript.length === 0 &&
      events.length === 0 ? (
        <EmptyObserverState />
      ) : connectionState === "connecting" &&
        events.length === 0 &&
        !hasTranscriptOverride ? (
        <SessionLoadingSkeleton />
      ) : (
        <div
          className={cn(
            rawRail.mode === "side"
              ? "mt-4 grid gap-4 xl:grid-cols-[minmax(0,1fr)_20rem]"
              : "mt-0",
            autoTail && "min-h-0 flex-1 overflow-hidden",
          )}
        >
          <AgentSessionTranscriptList
            liveStatusSlot={liveStatusSlot}
            agentAvatarUrl={agentAvatarUrl}
            agentName={agentName}
            agentPubkey={agentPubkey}
            channelId={channelId}
            emptyDescription={emptyDescription}
            emptyState={emptyState}
            items={transcript}
            profiles={profiles}
            contentContainerClassName={transcriptContentClassName}
            scrollScopeKey={`${agentPubkey}:${channelId ?? "all"}`}
            autoTail={autoTail}
            variant={transcriptVariant}
          />
          {rawRail.mode === "side" ? <RawEventRail events={events} /> : null}
        </div>
      )}

      {errorMessage ? (
        <p className="mt-4 inline-flex items-center gap-2 rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          <CircleAlert className="h-4 w-4" />
          {errorMessage}
        </p>
      ) : null}
    </>
  );
}

function SessionLoadingSkeleton() {
  return (
    <div className="mx-auto w-full max-w-3xl space-y-6 py-4">
      <div className="flex justify-end">
        <div className="max-w-[70%] space-y-2">
          <Skeleton className="h-4 w-48 rounded-lg" />
        </div>
      </div>
      <div className="space-y-2">
        <div className="flex items-center gap-2">
          <Skeleton className="h-5 w-5 rounded-full" />
          <Skeleton className="h-3 w-16 rounded-full" />
        </div>
        <div className="space-y-2">
          <Skeleton className="h-4 w-full rounded-lg" />
          <Skeleton className="h-4 w-[86%] rounded-lg" />
          <Skeleton className="h-4 w-[58%] rounded-lg" />
        </div>
      </div>
      <div className="space-y-2">
        <Skeleton className="h-4 w-44 rounded-lg" />
        <Skeleton className="h-4 w-[68%] rounded-lg" />
      </div>
    </div>
  );
}

function ObserverStatusBadge({ state }: { state: ConnectionState }) {
  const display =
    state === "open"
      ? { label: "Live", Icon: CircleDot, variant: "default" as const }
      : state === "connecting"
        ? { label: "Connecting", variant: "secondary" as const }
        : state === "error"
          ? {
              label: "Unavailable",
              Icon: XCircle,
              variant: "destructive" as const,
            }
          : state === "closed"
            ? { label: "Closed", Icon: Clock3, variant: "secondary" as const }
            : { label: "Idle", Icon: Clock3, variant: "secondary" as const };
  const StatusIcon = display.Icon;

  return (
    <Badge className="gap-1.5" variant={display.variant}>
      {StatusIcon ? (
        <StatusIcon aria-hidden className="h-4 w-4" />
      ) : (
        <Spinner aria-hidden className="h-4 w-4 border-2" />
      )}
      {display.label}
    </Badge>
  );
}

function EmptyObserverState() {
  return (
    <div className="mt-4 flex min-h-48 flex-col items-center justify-center px-6 py-8 text-center">
      <TerminalSquare className="mx-auto h-4 w-4 text-muted-foreground" />
      <p className="mt-3 text-sm font-medium">Observer not attached</p>
      <p className="mt-1 text-sm text-muted-foreground">
        The live feed is available for local agents started after this update.
      </p>
    </div>
  );
}
