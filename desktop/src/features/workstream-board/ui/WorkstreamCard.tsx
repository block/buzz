import * as React from "react";
import { Hash } from "lucide-react";
import type { BoardReference } from "@/features/workstream-board/lib/boardReferences";
import { boardReferenceKey } from "@/features/workstream-board/lib/boardReferences";

import type { ActiveChannelTurnSummary } from "@/features/agents/activeAgentTurnsStore";
import {
  useCanvasQuery,
  useChannelDetailsQuery,
} from "@/features/channels/hooks";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import {
  parseWorkstreamPullRequestReferences,
  useWorkstreamPullRequestStatuses,
} from "@/features/workstream-board/lib/workstreamPullRequestStatus";
import {
  parseWorkstreamWaitMessageLink,
  waitsForCard,
  type WorkstreamWait,
} from "@/features/workstream-board/lib/workstreamWaits";
import { buildWorkstreamCardViewModel } from "@/features/workstream-board/lib/workstreamCardViewModel";
import { WorkstreamPullRequests } from "@/features/workstream-board/ui/WorkstreamPullRequests";
import { WorkstreamWaits } from "@/features/workstream-board/ui/WorkstreamWaits";
import { WorkstreamAgentStatusPills } from "@/features/workstream-board/ui/WorkstreamAgentStatusPills";
import { classifyWorkstreamAgents } from "@/features/workstream-board/lib/workstreamAgentStatuses";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

type WorkstreamCardProps = {
  activeWorking?: ActiveChannelTurnSummary;
  channel: Channel;
  profiles?: UserProfileLookup;
  currentOwnerPubkey?: string;
  onWaitsChange?: (
    waits: readonly WorkstreamWait[],
    createdAt: string | null | undefined,
  ) => void;
  onSelect: (channelId: string) => void;
  discussionMode?: boolean;
  selectedReferenceKeys?: ReadonlySet<string>;
  replayReferenceKeys?: ReadonlySet<string>;
  replayFocusedReferenceKey?: string;
  replayReferenceStates?: ReadonlyMap<string, "live" | "changed">;
  historicalReplayReferences?: readonly BoardReference[];
  onToggleReference?: (reference: BoardReference) => void;
  onReferencesChange?: (references: readonly BoardReference[]) => void;
};

export function WorkstreamCard({
  activeWorking,
  channel,
  discussionMode = false,
  currentOwnerPubkey,
  onSelect,
  onWaitsChange,
  onToggleReference,
  onReferencesChange,
  profiles,
  replayReferenceKeys,
  replayFocusedReferenceKey,
  replayReferenceStates,
  historicalReplayReferences = [],
  selectedReferenceKeys,
}: WorkstreamCardProps) {
  const canvasQuery = useCanvasQuery(channel.id);
  const detailsQuery = useChannelDetailsQuery(channel.id);
  const viewModel = buildWorkstreamCardViewModel({
    canvasContent: canvasQuery.data?.content,
    isLoading: canvasQuery.isLoading,
    isError: canvasQuery.isError,
  });
  const references = React.useMemo(
    () =>
      viewModel.status === "ready"
        ? parseWorkstreamPullRequestReferences(viewModel.card.pullRequests)
        : [],
    [viewModel],
  );
  const pullRequestStates = useWorkstreamPullRequestStatuses(references);
  const waits = React.useMemo(
    () =>
      viewModel.status === "ready"
        ? waitsForCard(viewModel.card, references, pullRequestStates)
        : [],
    [pullRequestStates, references, viewModel],
  );
  React.useEffect(() => {
    onWaitsChange?.(waits, detailsQuery.data?.createdAt);
  }, [detailsQuery.data?.createdAt, onWaitsChange, waits]);
  const agentStatuses = React.useMemo(
    () =>
      viewModel.status === "ready"
        ? classifyWorkstreamAgents(
            viewModel.card.assignees,
            activeWorking,
            waits,
          )
        : { working: [], waiting: [] },
    [activeWorking, viewModel, waits],
  );
  const waitingOnPrincipal = waits.some(
    (wait) =>
      wait.actor.pubkey &&
      currentOwnerPubkey &&
      normalizePubkey(wait.actor.pubkey) ===
        normalizePubkey(currentOwnerPubkey),
  );
  const discussionReferences = React.useMemo<BoardReference[]>(() => {
    if (viewModel.status !== "ready") return [];
    const placement = {
      workstreamId: channel.id,
      workstreamLabel: channel.name,
      sectionId: "workstream",
      sectionLabel: "Workstream",
    };
    const result: BoardReference[] = [
      {
        kind: "workstream",
        identity: channel.id,
        snapshot: { label: channel.name, detail: viewModel.card.synopsis },
        placement,
      },
    ];
    for (const reference of references) {
      result.push({
        kind: "pull-request",
        identity: reference.identity,
        snapshot: { label: reference.rawUrl },
        placement: {
          ...placement,
          sectionId: "pull-requests",
          sectionLabel: "Pull requests",
        },
      });
    }
    for (const assignee of viewModel.card.assignees) {
      const identity = normalizePubkey(assignee.pubkey);
      if (/^[0-9a-f]{64}$/.test(identity))
        result.push({
          kind: "agent",
          identity,
          snapshot: { label: assignee.name },
          placement: {
            ...placement,
            sectionId: "agents",
            sectionLabel: "Agents",
          },
        });
    }
    result.push({
      kind: "agent-group",
      identity: `orchestrator:${normalizePubkey(viewModel.card.orchestrator.pubkey)}`,
      snapshot: {
        label: `${viewModel.card.orchestrator.name}'s team`,
        detail: "Orchestrator group",
      },
      placement: { ...placement, sectionId: "agents", sectionLabel: "Agents" },
    });
    for (const wait of waits) {
      const destination = parseWorkstreamWaitMessageLink(wait.message);
      if (!destination) continue;
      result.push({
        kind: "thread-blocker",
        identity: wait.key,
        destination: {
          channelId: destination.channelId,
          messageId: destination.messageId,
          ...(destination.threadRootId
            ? { threadRootId: destination.threadRootId }
            : {}),
        },
        snapshot: { label: wait.reason, detail: wait.actor.name },
        placement: {
          ...placement,
          sectionId: "blockers",
          sectionLabel: "Blockers",
        },
      });
    }
    return result;
  }, [channel.id, channel.name, references, viewModel, waits]);
  React.useEffect(() => {
    onReferencesChange?.(discussionReferences);
  }, [discussionReferences, onReferencesChange]);

  const workstreamReference = discussionReferences[0];
  const workstreamReferenceKey = workstreamReference
    ? boardReferenceKey(workstreamReference)
    : undefined;
  const workstreamIsCurrent =
    workstreamReferenceKey !== undefined &&
    replayFocusedReferenceKey === workstreamReferenceKey;

  return (
    <div
      aria-current={workstreamIsCurrent ? "true" : undefined}
      className={cn(
        "group relative w-full self-start overflow-hidden rounded-2xl border border-border/70 bg-muted/50 p-5 text-left text-foreground shadow-xs transition-all hover:-translate-y-0.5 hover:border-border hover:bg-muted/65 hover:shadow-md",
        waitingOnPrincipal && "border-t-4 border-t-amber-400/70",
        discussionReferences.some((reference) =>
          replayReferenceKeys?.has(boardReferenceKey(reference)),
        ) && "border-dashed border-2",
        workstreamIsCurrent &&
          "ring-2 ring-primary ring-offset-2 ring-offset-background",
      )}
      data-testid={`workstream-card-${channel.id}`}
    >
      <button
        className="absolute inset-0 z-0 rounded-2xl focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        onClick={() =>
          discussionMode
            ? workstreamReference && onToggleReference?.(workstreamReference)
            : onSelect(channel.id)
        }
        type="button"
      >
        <span className="sr-only">Open #{channel.name}</span>
      </button>
      {waitingOnPrincipal ? (
        <span className="pointer-events-none absolute right-4 top-0 z-20 rounded-b-md border border-t-0 border-amber-400/60 bg-muted px-2 py-1 text-3xs font-bold uppercase tracking-wider text-foreground">
          Priority
        </span>
      ) : null}
      <div className="pointer-events-none relative z-10 flex flex-col">
        <button
          className="pointer-events-auto flex max-w-[calc(100%-4rem)] items-center gap-1.5 text-xs font-semibold text-muted-foreground hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
          onClick={() =>
            discussionMode
              ? workstreamReference && onToggleReference?.(workstreamReference)
              : onSelect(channel.id)
          }
          type="button"
        >
          <Hash className="h-3.5 w-3.5 shrink-0" />
          <span className="truncate">{channel.name}</span>
        </button>
        {workstreamIsCurrent ? (
          <span
            className="mt-2 w-fit rounded-full border border-foreground/60 px-2 py-1 text-3xs font-bold uppercase tracking-wider"
            data-testid="current-workstream-reference"
          >
            Current reference
          </span>
        ) : null}
        {discussionMode ? (
          <div
            className="pointer-events-auto mt-3 flex flex-wrap gap-1.5"
            data-testid={`board-reference-options-${channel.id}`}
          >
            {discussionReferences.map((reference) => {
              const key = boardReferenceKey(reference);
              const selected = selectedReferenceKeys?.has(key) ?? false;
              const replayed = replayReferenceKeys?.has(key) ?? false;
              const replayState = replayReferenceStates?.get(key);
              const current =
                reference.kind !== "workstream" &&
                replayFocusedReferenceKey === key;
              return (
                <button
                  aria-pressed={selected}
                  className={cn(
                    "rounded-full border px-2 py-1 text-3xs",
                    selected && "border-primary bg-primary/15",
                    replayed && "border-dashed font-semibold",
                    current && "ring-2 ring-primary ring-offset-2",
                  )}
                  aria-current={current ? "true" : undefined}
                  key={key}
                  onClick={() => onToggleReference?.(reference)}
                  type="button"
                >
                  {reference.kind}: {reference.snapshot.label}
                  {replayed ? ` · Referenced · ${replayState}` : ""}
                  {current ? " · Current" : ""}
                </button>
              );
            })}
            {historicalReplayReferences.map((reference) => {
              const key = boardReferenceKey(reference);
              const current = replayFocusedReferenceKey === key;
              return (
                <div
                  aria-current={current ? "true" : undefined}
                  className={cn(
                    "rounded-md border border-dashed px-2 py-1 text-3xs font-semibold",
                    current && "ring-2 ring-primary ring-offset-2",
                  )}
                  data-testid="historical-reference-placeholder"
                  key={key}
                >
                  Historical {reference.placement.sectionLabel}:{" "}
                  {reference.snapshot.label}
                  {current ? " · Current" : ""}
                </div>
              );
            })}
            <button
              className="rounded-full border px-2 py-1 text-3xs font-semibold"
              onClick={() => onSelect(channel.id)}
              type="button"
            >
              Open
            </button>
          </div>
        ) : null}
        {viewModel.status === "ready" ? (
          <>
            <p className="mt-3 line-clamp-3 text-sm leading-relaxed text-foreground">
              {viewModel.card.synopsis}
            </p>
            <div className="flex flex-col gap-2 pt-4">
              <p className="truncate text-2xs text-muted-foreground">
                Orchestrator:{" "}
                <span className="text-foreground">
                  {viewModel.card.orchestrator.name}
                </span>
              </p>
              <WorkstreamAgentStatusPills
                profiles={profiles}
                statuses={agentStatuses}
              />
              {references.length > 0 ? (
                <WorkstreamPullRequests
                  references={references}
                  states={pullRequestStates}
                />
              ) : null}
              <WorkstreamWaits
                profiles={profiles}
                references={references}
                waits={waits}
              />
            </div>
          </>
        ) : viewModel.status === "loading" ? (
          <p className="mt-3 text-sm text-muted-foreground">Loading…</p>
        ) : (
          <div
            className="mt-3 flex flex-1 flex-col justify-center gap-1"
            data-testid="card-details-unavailable"
          >
            <p className="text-sm text-muted-foreground">
              Card details unavailable
            </p>
            {channel.description ? (
              <p className="line-clamp-2 text-xs text-muted-foreground/70">
                {channel.description}
              </p>
            ) : null}
          </div>
        )}
      </div>
    </div>
  );
}
