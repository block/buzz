import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  cancelScheduledMessage,
  listScheduledMessages,
  scheduleMessage,
  type ScheduleMessageInput,
  type ScheduledMessage,
} from "@/shared/api/scheduledMessages";

export const scheduledMessagesQueryKey = ["scheduled-messages"] as const;

/** Poll the pending queue at this cadence while the Scheduled view is open. */
export const SCHEDULED_MESSAGES_REFETCH_INTERVAL_MS = 15_000;

export function useScheduledMessagesQuery() {
  return useQuery({
    queryKey: scheduledMessagesQueryKey,
    queryFn: listScheduledMessages,
    refetchInterval: SCHEDULED_MESSAGES_REFETCH_INTERVAL_MS,
    refetchOnWindowFocus: true,
  });
}

/** Invalidate the queue everywhere (list view + any pending optimists). */
export function invalidateScheduledMessages(
  queryClient: ReturnType<typeof useQueryClient>,
) {
  return queryClient.invalidateQueries({
    queryKey: scheduledMessagesQueryKey,
  });
}

export function useScheduleMessageMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: ScheduleMessageInput) => scheduleMessage(input),
    onSuccess: () => invalidateScheduledMessages(queryClient),
  });
}

export function useCancelScheduledMessageMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => cancelScheduledMessage(id),
    onSuccess: (removed) => {
      queryClient.setQueryData<ScheduledMessage[]>(
        scheduledMessagesQueryKey,
        (current) =>
          (current ?? []).filter((message) => message.id !== removed.id),
      );
    },
  });
}
