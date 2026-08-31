import { RefreshCcw } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useChannelsQuery } from "@/features/channels/hooks";
import {
  useChannelTasks,
  useSetChannelTaskStatus,
} from "@/features/tasks/lib/useChannelTasks";
import { MyWorkView } from "@/features/tasks/ui/MyWorkView";
import { useIdentityQuery } from "@/shared/api/hooks";
import { Button } from "@/shared/ui/button";

/**
 * Community work supervision surface. Channel-task rows remain the durable
 * substrate; this screen projects them into an intervention-first queue and a
 * truthful selected-task brief. A channel filter narrows the same query used by
 * channel-local task panels rather than creating a second task implementation.
 */
export function TasksScreen() {
  const channelsQuery = useChannelsQuery();
  const channels = channelsQuery.data ?? [];
  const identityQuery = useIdentityQuery();
  const [selectedChannelId, setSelectedChannelId] = React.useState<
    string | null
  >(null);
  const tasks = useChannelTasks(selectedChannelId);
  const setStatus = useSetChannelTaskStatus();
  const channelNamesById = React.useMemo(
    () => new Map(channels.map((channel) => [channel.id, channel.name])),
    [channels],
  );
  const sortedChannels = React.useMemo(
    () =>
      [...channels].sort((left, right) => left.name.localeCompare(right.name)),
    [channels],
  );

  React.useEffect(() => {
    if (
      selectedChannelId !== null &&
      !channels.some((channel) => channel.id === selectedChannelId)
    ) {
      setSelectedChannelId(null);
    }
  }, [channels, selectedChannelId]);

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <header className="flex min-h-16 flex-wrap items-center gap-3 border-b bg-background px-5 py-3">
        <div className="min-w-0">
          <h1 className="text-lg font-semibold text-foreground">My Work</h1>
          <p className="text-sm text-muted-foreground">
            Requests, owned work, and verified task state in one place.
          </p>
        </div>
        <div className="ml-auto flex items-end gap-2">
          <label className="flex min-w-44 flex-col gap-1 text-xs font-medium text-muted-foreground">
            Channel
            <select
              className="h-9 rounded-md border border-input bg-background px-3 text-sm text-foreground shadow-xs outline-none focus-visible:ring-2 focus-visible:ring-ring"
              data-testid="channel-task-scope"
              onChange={(event) =>
                setSelectedChannelId(event.target.value || null)
              }
              value={selectedChannelId ?? ""}
            >
              <option value="">All channels</option>
              {sortedChannels.map((channel) => (
                <option key={channel.id} value={channel.id}>
                  #{channel.name}
                </option>
              ))}
            </select>
          </label>
          <Button
            aria-label="Refresh My Work"
            disabled={tasks.isFetching}
            onClick={() => void tasks.refetch()}
            size="icon"
            type="button"
            variant="outline"
          >
            <RefreshCcw
              className={tasks.isFetching ? "animate-spin" : undefined}
            />
          </Button>
        </div>
      </header>
      <MyWorkView
        channelNamesById={channelNamesById}
        currentPubkey={identityQuery.data?.pubkey}
        error={tasks.error}
        isLoading={tasks.isLoading}
        isStatusPending={setStatus.isPending}
        onRetry={() => void tasks.refetch()}
        onSetStatus={(task, status) => {
          setStatus.mutate(
            { taskId: task.id, status },
            {
              onError: (error: unknown) => {
                toast.error(
                  error instanceof Error
                    ? error.message
                    : "Could not update the task",
                );
              },
            },
          );
        }}
        tasks={tasks.data ?? []}
      />
    </div>
  );
}
