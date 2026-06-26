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
import { AgentSessionTranscriptList } from "./AgentSessionTranscriptList";
import { RawEventRail } from "./RawEventRail";
import type {
  ConnectionState,
  ObserverEvent,
  TranscriptItem,
} from "./agentSessionTypes";
import { shorten } from "./agentSessionUtils";
import { useObserverEvents, useAgentTranscript } from "./useObserverEvents";

type ManagedAgentSessionPanelProps = {
  agent: Pick<ManagedAgent, "pubkey" | "name" | "status">;
  channelId?: string | null;
  className?: string;
  emptyDescription?: string;
  showHeader?: boolean;
  showRaw?: boolean;
  profiles?: UserProfileLookup;
};

export function ManagedAgentSessionPanel({
  agent,
  channelId = null,
  className,
  emptyDescription = "Mention this agent in a channel to watch the next turn.",
  showHeader = true,
  showRaw = true,
  profiles,
}: ManagedAgentSessionPanelProps) {
  const hasObserver = isManagedAgentActive(agent);
  const { connectionState, errorMessage, events } = useObserverEvents(
    hasObserver,
    agent.pubkey,
  );
  const transcript = useAgentTranscript(hasObserver, agent.pubkey);

  const scopedTranscript = React.useMemo(
    () =>
      channelId
        ? transcript.filter((item) => item.channelId === channelId)
        : transcript,
    [channelId, transcript],
  );

  const scopedEvents = React.useMemo(
    () =>
      channelId
        ? events.filter((event) => event.channelId === channelId)
        : events,
    [channelId, events],
  );

  const latestSessionId = React.useMemo(() => {
    for (let i = scopedEvents.length - 1; i >= 0; i--) {
      if (scopedEvents[i].sessionId) return scopedEvents[i].sessionId;
    }
    return null;
  }, [scopedEvents]);

  return (
    <section
      className={cn(
        "rounded-lg border border-border/70 bg-background/80 p-4 shadow-xs",
        className,
      )}
    >
      {showHeader ? (
        <SessionHeader
          connectionState={connectionState}
          eventCount={scopedEvents.length}
          hasObserver={hasObserver}
          latestSessionId={latestSessionId}
        />
      ) : null}

      <SessionBody
        agentName={agent.name}
        connectionState={connectionState}
        emptyDescription={emptyDescription}
        errorMessage={errorMessage}
        events={scopedEvents}
        hasObserver={hasObserver}
        profiles={profiles}
        showRaw={showRaw}
        transcript={scopedTranscript}
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
  agentName,
  connectionState,
  emptyDescription,
  errorMessage,
  events,
  hasObserver,
  profiles,
  showRaw,
  transcript,
}: {
  agentName: string;
  connectionState: ConnectionState;
  emptyDescription: string;
  errorMessage: string | null;
  events: ObserverEvent[];
  hasObserver: boolean;
  profiles?: UserProfileLookup;
  showRaw: boolean;
  transcript: TranscriptItem[];
}) {
  return (
    <>
      {!hasObserver ? (
        <EmptyObserverState />
      ) : connectionState === "connecting" && events.length === 0 ? (
        <SessionLoadingSkeleton />
      ) : (
        <div
          className={cn(
            showRaw
              ? "mt-4 grid gap-4 xl:grid-cols-[minmax(0,1fr)_20rem]"
              : "mt-0",
          )}
        >
          <AgentSessionTranscriptList
            agentName={agentName}
            emptyDescription={emptyDescription}
            items={transcript}
            profiles={profiles}
          />
          {showRaw ? <RawEventRail events={events} /> : null}
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
