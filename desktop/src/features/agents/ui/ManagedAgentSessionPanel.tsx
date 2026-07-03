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
  resolveDisplayEvents,
  resolveRawRailLayout,
  scopeByChannel,
} from "./agentSessionPanelLayout";
import { shorten } from "./agentSessionUtils";
import { useObserverEvents, useAgentTranscript } from "./useObserverEvents";

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
}: ManagedAgentSessionPanelProps) {
  const hasObserver = isManagedAgentActive(agent);
  const { connectionState, errorMessage, events } = useObserverEvents(
    hasObserver,
    agent.pubkey,
  );
  const transcript = useAgentTranscript(hasObserver, agent.pubkey);

  const scopedTranscript = React.useMemo(
    () => scopeByChannel(transcript, channelId),
    [channelId, transcript],
  );

  const displayTranscript = transcriptOverride ?? scopedTranscript;

  const scopedEvents = React.useMemo(
    () => scopeByChannel(events, channelId),
    [channelId, events],
  );
  const displayEvents = React.useMemo(
    () => resolveDisplayEvents(scopedEvents, rawEventsOverride),
    [rawEventsOverride, scopedEvents],
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
      {!hasObserver && !hasTranscriptOverride ? (
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
