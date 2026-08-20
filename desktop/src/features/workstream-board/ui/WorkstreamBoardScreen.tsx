import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useActiveAgentTurnsByChannel } from "@/features/agents/activeAgentTurnsStore";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { filterWorkstreamChannels } from "@/features/workstream-board/lib/discoverWorkstreamChannels";
import { getActiveWorkstreamTurns } from "@/features/workstream-board/lib/activeWorkstreamTurns";
import { WorkstreamCard } from "@/features/workstream-board/ui/WorkstreamCard";
import { Button } from "@/shared/ui/button";
import { PageHeader } from "@/shared/ui/PageHeader";

const WORKSTREAM_CARD_GRID_CLASS =
  "grid grid-cols-1 gap-3 [@container(min-width:38rem)]:grid-cols-2 [@container(min-width:54rem)]:grid-cols-3";

export function WorkstreamBoardScreen() {
  const { goChannel } = useAppNavigation();
  const channelsQuery = useChannelsQuery();
  const channels = channelsQuery.data ?? [];
  const workstreamChannels = filterWorkstreamChannels(channels);
  const activeTurnsByChannel = useActiveAgentTurnsByChannel();
  const activeWorkstreamTurns = React.useMemo(
    () => getActiveWorkstreamTurns(workstreamChannels, activeTurnsByChannel),
    [activeTurnsByChannel, workstreamChannels],
  );
  const activeTurnsByChannelId = React.useMemo(
    () => new Map(activeWorkstreamTurns.map((turn) => [turn.channelId, turn])),
    [activeWorkstreamTurns],
  );
  const activeAgentPubkeys = React.useMemo(
    () => activeWorkstreamTurns.flatMap((turn) => turn.agentPubkeys),
    [activeWorkstreamTurns],
  );
  const activeProfilesQuery = useUsersBatchQuery(activeAgentPubkeys);
  const activeProfiles = activeProfilesQuery.data?.profiles;

  return (
    <div
      className="relative flex min-h-0 flex-1 overflow-hidden"
      data-testid="workstream-board-view"
    >
      <div
        className="flex min-h-0 flex-1 flex-col overflow-y-auto overflow-x-hidden overscroll-contain px-4 py-7 sm:px-6 sm:py-8"
        data-scroll-restoration-id="workstream-board-list"
      >
        <div className="mx-auto w-full max-w-6xl space-y-8 [container-type:inline-size]">
          <PageHeader
            description="Live canvases for active workstream channels."
            title="Workstream Board"
          />

          {channelsQuery.isLoading ? (
            <p className="text-sm text-muted-foreground">
              Loading workstreams…
            </p>
          ) : channelsQuery.isError ? (
            <div className="flex flex-col items-center justify-center gap-2 py-16 text-muted-foreground">
              <p className="text-sm text-red-400">Failed to load channels</p>
              <Button
                onClick={() => void channelsQuery.refetch()}
                size="sm"
                variant="outline"
              >
                Retry
              </Button>
            </div>
          ) : workstreamChannels.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No workstream channels found. Channels named "loganj-ws-…" will
              appear here.
            </p>
          ) : (
            <div className={WORKSTREAM_CARD_GRID_CLASS}>
              {workstreamChannels.map((channel) => (
                <WorkstreamCard
                  activeWorking={activeTurnsByChannelId.get(channel.id)}
                  channel={channel}
                  key={channel.id}
                  profiles={activeProfiles}
                  onSelect={(channelId) => void goChannel(channelId)}
                />
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
