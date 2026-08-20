import { Hash } from "lucide-react";

import type { ActiveChannelTurnSummary } from "@/features/agents/activeAgentTurnsStore";
import { useCanvasQuery } from "@/features/channels/hooks";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { buildWorkstreamCardViewModel } from "@/features/workstream-board/lib/workstreamCardViewModel";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { WorkstreamWorkingIndicator } from "@/features/workstream-board/ui/WorkstreamWorkingIndicator";

type WorkstreamCardProps = {
  activeWorking?: ActiveChannelTurnSummary;
  channel: Channel;
  profiles?: UserProfileLookup;
  onSelect: (channelId: string) => void;
};

export function WorkstreamCard({
  activeWorking,
  channel,
  onSelect,
  profiles,
}: WorkstreamCardProps) {
  const canvasQuery = useCanvasQuery(channel.id);
  const viewModel = buildWorkstreamCardViewModel({
    canvasContent: canvasQuery.data?.content,
    isLoading: canvasQuery.isLoading,
    isError: canvasQuery.isError,
  });

  return (
    <div
      className={cn(
        "group relative min-h-48 w-full overflow-hidden rounded-2xl border border-border/70 bg-muted/50 p-5 text-left text-foreground shadow-xs transition-all hover:-translate-y-0.5 hover:border-border hover:bg-muted/65 hover:shadow-md",
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

      <div className="pointer-events-none relative z-10 flex h-full min-h-40 flex-col">
        <div className="flex items-center gap-1.5 text-xs font-semibold text-muted-foreground">
          <Hash className="h-3.5 w-3.5 shrink-0" />
          <span className="truncate">{channel.name}</span>
        </div>

        {viewModel.status === "ready" ? (
          <>
            <p className="mt-3 line-clamp-3 text-sm leading-relaxed text-foreground">
              {viewModel.card.synopsis}
            </p>
            <div className="mt-auto flex flex-col gap-2 pt-4">
              <p className="truncate text-2xs text-muted-foreground">
                Orchestrator:{" "}
                <span className="text-foreground">
                  {viewModel.card.orchestrator.name}
                </span>
              </p>
              {viewModel.card.assignees.length > 0 ? (
                <div className="flex flex-wrap gap-1">
                  {viewModel.card.assignees.map((assignee) => (
                    <span
                      className="rounded-full border border-border/65 bg-background/80 px-2 py-0.5 text-2xs text-muted-foreground"
                      key={assignee.pubkey}
                    >
                      {assignee.name}
                    </span>
                  ))}
                </div>
              ) : null}
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
        {activeWorking ? (
          <WorkstreamWorkingIndicator
            activeWorking={activeWorking}
            profiles={profiles}
          />
        ) : null}
      </div>
    </div>
  );
}
