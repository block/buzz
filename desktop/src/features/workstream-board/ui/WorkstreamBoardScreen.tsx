import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useActiveAgentTurnsByChannel } from "@/features/agents/activeAgentTurnsStore";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { filterWorkstreamChannels } from "@/features/workstream-board/lib/discoverWorkstreamChannels";
import { getActiveWorkstreamTurns } from "@/features/workstream-board/lib/activeWorkstreamTurns";
import {
  sortWorkstreamCards,
  type WorkstreamWait,
} from "@/features/workstream-board/lib/workstreamWaits";
import { WorkstreamCard } from "@/features/workstream-board/ui/WorkstreamCard";
import { useIdentityQuery } from "@/shared/api/hooks";
import { Button } from "@/shared/ui/button";
import { PageHeader } from "@/shared/ui/PageHeader";

const WORKSTREAM_CARD_GRID_CLASS =
  "grid grid-cols-1 gap-3 [@container(min-width:38rem)]:grid-cols-2 [@container(min-width:54rem)]:grid-cols-3";
type CardMetadata = {
  waits: readonly WorkstreamWait[];
  createdAt: string | null | undefined;
};

function sameMetadata(
  left: CardMetadata | undefined,
  right: CardMetadata,
): boolean {
  return (
    left?.createdAt === right.createdAt &&
    JSON.stringify(left?.waits) === JSON.stringify(right.waits)
  );
}

export function WorkstreamBoardScreen() {
  const { goChannel } = useAppNavigation();
  const identityQuery = useIdentityQuery();
  const channelsQuery = useChannelsQuery();
  const workstreamChannels = filterWorkstreamChannels(channelsQuery.data ?? []);
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
  const [metadataByChannel, setMetadataByChannel] = React.useState<
    ReadonlyMap<string, CardMetadata>
  >(() => new Map());
  const onWaitsChange = React.useCallback(
    (
      channelId: string,
      waits: readonly WorkstreamWait[],
      createdAt: string | null | undefined,
    ) => {
      const next = { waits, createdAt };
      setMetadataByChannel((current) => {
        if (sameMetadata(current.get(channelId), next)) return current;
        const updated = new Map(current);
        updated.set(channelId, next);
        return updated;
      });
    },
    [],
  );
  const sortedChannels = React.useMemo(
    () =>
      sortWorkstreamCards(
        workstreamChannels.map((channel) => ({
          channelId: channel.id,
          channelName: channel.name,
          createdAt: metadataByChannel.get(channel.id)?.createdAt,
          waits: metadataByChannel.get(channel.id)?.waits ?? [],
          activeWorking: activeTurnsByChannelId.get(channel.id),
          channel,
        })),
        identityQuery.data?.pubkey,
      ).map((entry) => entry.channel),
    [
      activeTurnsByChannelId,
      identityQuery.data?.pubkey,
      metadataByChannel,
      workstreamChannels,
    ],
  );

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
          ) : sortedChannels.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No workstream channels found. Channels named "loganj-ws-…" will
              appear here.
            </p>
          ) : (
            <div className={WORKSTREAM_CARD_GRID_CLASS}>
              {sortedChannels.map((channel) => (
                <WorkstreamCard
                  activeWorking={activeTurnsByChannelId.get(channel.id)}
                  channel={channel}
                  key={channel.id}
                  onSelect={(channelId) => void goChannel(channelId)}
                  onWaitsChange={(waits, createdAt) =>
                    onWaitsChange(channel.id, waits, createdAt)
                  }
                  profiles={activeProfilesQuery.data?.profiles}
                />
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
