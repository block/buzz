import { useEffect, useRef } from "react";
import { useIsFetching, useQuery, useQueryClient } from "@tanstack/react-query";
import { channelMessagesKey, channelWindowKey } from "../lib/messageQueryKeys";
import {
  emptyChannelWindowStore,
  type ChannelWindowStore,
} from "../lib/channelWindowStore";
import { refreshChannelWindowMessages } from "../lib/projectChannelWindow";
import { Button } from "@/shared/ui/button";

/** Recovery is explicit: keep reading, retry preserving history, or load latest. */
export function HistoryRefreshNotice({
  channelId,
  onLoadLatest,
}: {
  channelId?: string | null;
  onLoadLatest: () => void;
}) {
  const client = useQueryClient();
  const { data: window } = useQuery<ChannelWindowStore>({
    queryKey: channelWindowKey(channelId ?? "none"),
    enabled: false,
    staleTime: Infinity,
  });
  const isRefreshing =
    useIsFetching({
      queryKey: channelMessagesKey(channelId ?? "none"),
      exact: true,
    }) > 0;
  const scopeRef = useRef({ channelId, active: true });
  if (scopeRef.current.channelId !== channelId) {
    scopeRef.current.active = false;
    scopeRef.current = { channelId, active: true };
  }
  const scope = scopeRef.current;
  useEffect(() => {
    scope.active = true;
    return () => {
      scope.active = false;
    };
  }, [scope]);
  if (!channelId || !window?.refreshError) return null;
  const retry = async (latest: boolean) => {
    const token = latest ? crypto.randomUUID() : undefined;
    client.setQueryData<ChannelWindowStore>(
      channelWindowKey(channelId),
      (current) => ({
        ...(current ?? emptyChannelWindowStore()),
        refreshLatestOnly: token,
      }),
    );
    try {
      await refreshChannelWindowMessages(client, channelId);
    } catch {
      // Exhausted query retries are already projected into refreshError.
      // The recovery button owns this promise, so contain the rejection here.
      return;
    } finally {
      // The observer can unmount before hydration/invalidation starts a fetch.
      // Retire an unclaimed token too, without clearing a newer button click.
      if (token)
        client.setQueryData<ChannelWindowStore>(
          channelWindowKey(channelId),
          (current) =>
            current?.refreshLatestOnly === token
              ? { ...current, refreshLatestOnly: undefined }
              : current,
        );
    }
    const refreshed = client.getQueryData<ChannelWindowStore>(
      channelWindowKey(channelId),
    );
    // Failed recovery leaves the reader where they were. Do not wait for a
    // new tail id: an unchanged head still needs to honor explicit Load latest.
    if (
      latest &&
      scope.active &&
      refreshed?.pages.length &&
      !refreshed.refreshError
    )
      onLoadLatest();
  };
  return (
    <div
      role="status"
      data-testid="history-refresh-error"
      className="pointer-events-auto flex max-w-full flex-wrap items-center gap-2 rounded-xl border bg-background/95 px-3 py-2 text-sm shadow-sm"
    >
      <span>{window.refreshError}</span>
      <Button
        size="sm"
        variant="outline"
        disabled={isRefreshing}
        onClick={() => void retry(false)}
      >
        Retry
      </Button>
      <Button
        size="sm"
        variant="outline"
        disabled={isRefreshing}
        onClick={() => void retry(true)}
      >
        Load latest
      </Button>
    </div>
  );
}
