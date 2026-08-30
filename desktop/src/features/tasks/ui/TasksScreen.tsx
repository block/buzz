import * as React from "react";

import { useChannelsQuery } from "@/features/channels/hooks";
import { ChannelTaskList } from "@/features/tasks/ui/ChannelTaskList";

/**
 * Community-wide task view with an explicit channel scope. The all-channels
 * state keeps channel identity on every row; choosing a channel reuses the
 * same scoped list shown inside that channel's auxiliary panel.
 */
export function TasksScreen() {
  const channels = useChannelsQuery().data ?? [];
  const [selectedChannelId, setSelectedChannelId] = React.useState<
    string | null
  >(null);
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
    <div className="flex h-full flex-col gap-4 overflow-y-auto p-4">
      <header className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold">Channel tasks</h1>
          <p className="text-sm text-muted-foreground">
            Review every channel together or focus on one channel. Changes are
            visible to every member and on mobile.
          </p>
        </div>
        <label className="flex min-w-48 flex-col gap-1 text-xs font-medium text-muted-foreground">
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
      </header>
      <ChannelTaskList
        channelId={selectedChannelId}
        channelNamesById={channelNamesById}
      />
    </div>
  );
}
