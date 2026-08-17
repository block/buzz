import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  cancelReminder,
  completeReminder,
  createReminder,
  fetchReminders,
  snoozeReminder,
} from "@/features/reminders/lib/reminderService";
import { countDue } from "@/features/reminders/lib/reminderFilters";
import type {
  Reminder,
  ReminderTarget,
} from "@/features/reminders/lib/reminderTypes";

export const remindersQueryKey = (pubkey: string) =>
  ["reminders", pubkey] as const;

/** Re-exported so the inbox badge has one import for the due count. */
export const countDueReminders = countDue;

/**
 * The single source of truth for a user's reminders. Badge, channel overlay,
 * panel, and fire-on-due detection all read this one query, so invalidating it
 * (see {@link useReminderMutations}) keeps every surface consistent.
 */
export function useRemindersQuery(pubkey: string | undefined) {
  return useQuery({
    enabled: Boolean(pubkey),
    queryKey: remindersQueryKey(pubkey ?? ""),
    queryFn: () => fetchReminders(pubkey ?? ""),
    staleTime: 30_000,
  });
}

/**
 * The due-reminder contribution to the in-app Inbox nav badge. Reminders are a
 * separate stream from the feed badge machinery, so the count is summed in at
 * the AppShell wiring point rather than threaded through homeBadge.ts. Reads
 * the shared query above, so the useReminderNotifications poll's invalidate
 * keeps it live and countDue re-evaluates as reminders cross due. The caller
 * adds this raw (no isHomeActive suppression), mirroring the inbox filter
 * badge, which persists while the Inbox is open.
 *
 * `enabled` mirrors the homeBadgeEnabled contract: when the home badge toggle is
 * off, the feed contribution returns 0, so the reminder add must too — otherwise
 * a disabled badge would still show a reminder `(1)`.
 */
export function useDueReminderBadgeCount(
  pubkey: string | undefined,
  enabled: boolean,
): number {
  const remindersQuery = useRemindersQuery(pubkey);
  return enabled ? countDueReminders(remindersQuery.data ?? []) : 0;
}

/**
 * Wraps every reminder write so the shared query is invalidated on success —
 * the consistency spine the panel/badge/overlay all depend on. A mutation that
 * skipped invalidation would leave those surfaces stale until the next refetch.
 */
export function useReminderMutations(pubkey: string) {
  const queryClient = useQueryClient();
  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: remindersQueryKey(pubkey) });

  const create = useMutation({
    mutationFn: (input: {
      target: ReminderTarget;
      notBefore: number;
      note?: string;
    }) => createReminder(input.target, input.notBefore, input.note),
    onSuccess: invalidate,
  });
  const complete = useMutation({
    mutationFn: (reminder: Reminder) => completeReminder(pubkey, reminder),
    onSuccess: invalidate,
  });
  const snooze = useMutation({
    mutationFn: (input: { reminder: Reminder; notBefore: number }) =>
      snoozeReminder(pubkey, input.reminder, input.notBefore),
    onSuccess: invalidate,
  });
  const cancel = useMutation({
    mutationFn: (reminder: Reminder) => cancelReminder(pubkey, reminder),
    onSuccess: invalidate,
  });

  return { create, complete, snooze, cancel };
}
