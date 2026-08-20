import * as React from "react";
import { Hash } from "lucide-react";

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
};

export function WorkstreamCard({
  activeWorking,
  channel,
  currentOwnerPubkey,
  onSelect,
  onWaitsChange,
  profiles,
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

  return (
    <div
      className={cn(
        "group relative w-full self-start overflow-hidden rounded-2xl border border-border/70 bg-muted/50 p-5 text-left text-foreground shadow-xs transition-all hover:-translate-y-0.5 hover:border-border hover:bg-muted/65 hover:shadow-md",
        waitingOnPrincipal && "border-t-4 border-t-amber-400/70",
      )}
      data-testid={`workstream-card-${channel.id}`}
    >
      <button
        className="absolute inset-0 z-0 rounded-2xl focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        onClick={() => onSelect(channel.id)}
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
          onClick={() => onSelect(channel.id)}
          type="button"
        >
          <Hash className="h-3.5 w-3.5 shrink-0" />
          <span className="truncate">{channel.name}</span>
        </button>
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
