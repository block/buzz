import * as React from "react";
import {
  CheckCircle2,
  CircleAlert,
  CircleDot,
  Clock3,
  Pencil,
  TerminalSquare,
  XCircle,
} from "lucide-react";

import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import {
  buildMissionJournal,
  normalizeActivityEvents,
  type MissionJournal,
} from "@/features/agents/activityLedger";
import {
  applyValidatedJournalAuthority,
  journalAuthorityCorrelationId,
} from "@/features/agents/activityLedgerAuthority";
import {
  getJournalAuthorityArtifacts,
  upsertJournalVerification,
  upsertOwnerJournalOverride,
  type JournalAuthorityArtifact,
} from "@/shared/api/tauriArchive";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ManagedAgent } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { useNow } from "@/shared/lib/useNow";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Skeleton } from "@/shared/ui/skeleton";
import { Spinner } from "@/shared/ui/spinner";
import { Textarea } from "@/shared/ui/textarea";
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
  scopeByChannel,
} from "./agentSessionPanelLayout";
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
  const displayTranscript = transcriptOverride ?? derivedTranscript;

  const displayEvents = React.useMemo(
    () => resolveDisplayEvents(combinedEvents, rawEventsOverride),
    [rawEventsOverride, combinedEvents],
  );

  const latestSessionId = React.useMemo(
    () => deriveLatestSessionId(displayEvents),
    [displayEvents],
  );
  const journalAsOf = useNow(60_000);
  const latestJournal = React.useMemo(
    () =>
      buildMissionJournal(normalizeActivityEvents(combinedEvents), {
        asOf: new Date(journalAsOf),
      }),
    [combinedEvents, journalAsOf],
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

      {latestJournal.eventCount > 0 ? (
        <MissionJournalSummary journal={latestJournal} />
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

function journalStatusLabel(status: MissionJournal["status"]): string {
  switch (status) {
    case "in_progress":
      return "In progress";
    case "ended_unverified":
      return "Ended, proof needed";
    case "incomplete":
      return "Incomplete";
    case "completed":
      return "Execution ended";
    case "failed":
      return "Failed";
    case "observed":
      return "Observed";
  }
}

function MissionJournalSummary({ journal }: { journal: MissionJournal }) {
  const [artifacts, setArtifacts] = React.useState<JournalAuthorityArtifact[]>(
    [],
  );
  const [mode, setMode] = React.useState<"summary" | "verify" | null>(null);
  const [summary, setSummary] = React.useState(journal.summary);
  const [receiptRef, setReceiptRef] = React.useState("");
  const [saving, setSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  const reloadAuthority = React.useCallback(async () => {
    const current = await getJournalAuthorityArtifacts(journal.id);
    setArtifacts(current);
  }, [journal.id]);

  React.useEffect(() => {
    let cancelled = false;
    setArtifacts([]);
    setMode(null);
    setSummary(journal.summary);
    setReceiptRef("");
    setError(null);
    getJournalAuthorityArtifacts(journal.id)
      .then((current) => {
        if (!cancelled) setArtifacts(current);
      })
      .catch((loadError) => {
        if (!cancelled) {
          setError(
            loadError instanceof Error
              ? loadError.message
              : "Owner journal proof could not be loaded.",
          );
        }
      });
    return () => {
      cancelled = true;
    };
  }, [journal.id, journal.summary]);

  const authorizedJournal = React.useMemo(
    () => applyValidatedJournalAuthority(journal, artifacts),
    [artifacts, journal],
  );
  const receiptedSourceIds = React.useMemo(
    () => [
      ...new Set(
        journal.events
          .filter((event) => event.proofState === "RECEIPTED")
          .map((event) => event.provenance.sourceEventId)
          .filter(
            (id): id is string =>
              typeof id === "string" && /^[0-9a-f]{64}$/i.test(id),
          ),
      ),
    ],
    [journal.events],
  );
  const saveSummary = async () => {
    if (!summary.trim() || saving) return;
    setSaving(true);
    setError(null);
    try {
      await upsertOwnerJournalOverride({
        journalId: journal.id,
        correlationId: journal.correlationId,
        summary: summary.trim(),
      });
      await reloadAuthority();
      setMode(null);
    } catch (saveError) {
      setError(
        saveError instanceof Error
          ? saveError.message
          : "The owner summary was not saved.",
      );
    } finally {
      setSaving(false);
    }
  };

  const saveVerification = async () => {
    if (!receiptRef.trim() || receiptedSourceIds.length === 0 || saving) return;
    setSaving(true);
    setError(null);
    try {
      await upsertJournalVerification({
        journalId: journal.id,
        correlationId: journalAuthorityCorrelationId(journal),
        receiptRef: receiptRef.trim(),
        sourceEventIds: receiptedSourceIds,
      });
      await reloadAuthority();
      setMode(null);
      setReceiptRef("");
    } catch (saveError) {
      setError(
        saveError instanceof Error
          ? saveError.message
          : "The verification was not saved.",
      );
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="mt-3 rounded-md border border-border/60 bg-muted/30 px-3 py-2">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          Mission journal
        </span>
        <Badge variant="secondary">
          {journalStatusLabel(authorizedJournal.status)}
        </Badge>
        <Badge variant="outline">{authorizedJournal.proofState}</Badge>
        {authorizedJournal.claimedCompletionWithoutEvidence ? (
          <Badge variant="destructive">Evidence gap</Badge>
        ) : null}
      </div>
      <p className="mt-2 text-sm text-foreground">
        {authorizedJournal.summary}
      </p>
      {authorizedJournal.summarySource === "owner" ? (
        <p className="mt-1 text-xs text-muted-foreground">Owner edited</p>
      ) : null}

      {mode === "summary" ? (
        <div className="mt-3 space-y-2">
          <Textarea
            aria-label="Owner journal summary"
            value={summary}
            onChange={(event) => setSummary(event.target.value)}
          />
          <div className="flex gap-2">
            <Button
              size="xs"
              disabled={saving || !summary.trim()}
              onClick={saveSummary}
            >
              Save owner summary
            </Button>
            <Button size="xs" variant="ghost" onClick={() => setMode(null)}>
              Cancel
            </Button>
          </div>
        </div>
      ) : null}

      {mode === "verify" ? (
        <div className="mt-3 space-y-2">
          <Input
            aria-label="Verification receipt reference"
            placeholder="Receipt or independent check reference"
            value={receiptRef}
            onChange={(event) => setReceiptRef(event.target.value)}
          />
          {receiptedSourceIds.length === 0 ? (
            <p className="text-xs text-destructive">
              This journal has no receipted tool evidence to verify.
            </p>
          ) : null}
          <div className="flex gap-2">
            <Button
              size="xs"
              disabled={
                saving || !receiptRef.trim() || receiptedSourceIds.length === 0
              }
              onClick={saveVerification}
            >
              Record owner verification
            </Button>
            <Button size="xs" variant="ghost" onClick={() => setMode(null)}>
              Cancel
            </Button>
          </div>
        </div>
      ) : null}

      {mode == null ? (
        <div className="mt-2 flex flex-wrap gap-1">
          <Button size="xs" variant="ghost" onClick={() => setMode("summary")}>
            <Pencil /> Edit summary
          </Button>
          <Button
            size="xs"
            variant="ghost"
            disabled={authorizedJournal.proofState === "VERIFIED"}
            onClick={() => setMode("verify")}
          >
            <CheckCircle2 /> Verify receipt
          </Button>
        </div>
      ) : null}
      {error ? <p className="mt-2 text-xs text-destructive">{error}</p> : null}
    </div>
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
