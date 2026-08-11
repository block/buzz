import * as React from "react";
import { toast } from "sonner";

import {
  MAX_PINNED_MESSAGES,
  useSetPinnedMessagesMutation,
  usePinnedMessagesQuery,
} from "@/features/channels/hooks";

/**
 * Shared pin/unpin controller for a single channel/DM.
 *
 * Pins are channel/DM-scoped (not per reply-thread), capped at
 * `MAX_PINNED_MESSAGES`, and any member can pin or unpin — there is no
 * owner/admin gate here. `set_pinned_messages` is a full-replace command, so
 * `pin`/`unpin` compute the new complete list client-side before calling it.
 */
export function usePinnedMessagesActions(channelId: string | null) {
  const query = usePinnedMessagesQuery(channelId);
  const mutation = useSetPinnedMessagesMutation(channelId);
  const pinnedEventIds = React.useMemo(() => query.data ?? [], [query.data]);

  const isPinned = React.useCallback(
    (eventId: string) => pinnedEventIds.includes(eventId),
    [pinnedEventIds],
  );

  const pin = React.useCallback(
    (eventId: string) => {
      if (pinnedEventIds.includes(eventId)) return;
      if (pinnedEventIds.length >= MAX_PINNED_MESSAGES) {
        toast.error(
          `Only ${MAX_PINNED_MESSAGES} messages can be pinned at once. Unpin one first.`,
        );
        return;
      }
      mutation.mutate([...pinnedEventIds, eventId], {
        onError: (error) => {
          toast.error(
            error instanceof Error ? error.message : "Failed to pin message",
          );
        },
      });
    },
    [mutation, pinnedEventIds],
  );

  const unpin = React.useCallback(
    (eventId: string) => {
      if (!pinnedEventIds.includes(eventId)) return;
      mutation.mutate(
        pinnedEventIds.filter((id) => id !== eventId),
        {
          onError: (error) => {
            toast.error(
              error instanceof Error
                ? error.message
                : "Failed to unpin message",
            );
          },
        },
      );
    },
    [mutation, pinnedEventIds],
  );

  return {
    pinnedEventIds,
    isPinned,
    pin,
    unpin,
    isLoading: query.isLoading,
  };
}
