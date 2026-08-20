import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import { filterWorkstreamChannels } from "@/features/workstream-board/lib/discoverWorkstreamChannels";
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
                  channel={channel}
                  key={channel.id}
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
