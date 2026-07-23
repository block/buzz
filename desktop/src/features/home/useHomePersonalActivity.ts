import * as React from "react";

import { useHomeDrafts } from "@/features/home/useHomeDrafts";
import {
  countDueReminders,
  useRemindersQuery,
} from "@/features/reminders/hooks";
import { groupReminders } from "@/features/reminders/lib/reminderFilters";

type UseHomePersonalActivityOptions = {
  activityEnabled: boolean;
  allowMixedSelection: boolean;
  currentPubkey?: string;
  isDrafts: boolean;
  isNarrowHomeViewport: boolean;
  isReminders: boolean;
  viewportWidthPx: number;
};

export function useHomePersonalActivity({
  activityEnabled,
  allowMixedSelection,
  currentPubkey,
  isDrafts,
  isNarrowHomeViewport,
  isReminders,
  viewportWidthPx,
}: UseHomePersonalActivityOptions) {
  const remindersQuery = useRemindersQuery(currentPubkey);
  const dueReminderCount = countDueReminders(remindersQuery.data ?? []);
  const pendingReminders = React.useMemo(
    () =>
      groupReminders(remindersQuery.data ?? []).flatMap(
        (group) => group.reminders,
      ),
    [remindersQuery.data],
  );
  const [selectedReminderId, selectReminder] = React.useState<string | null>(
    null,
  );
  const selectedReminder =
    pendingReminders.find((reminder) => reminder.id === selectedReminderId) ??
    null;

  React.useEffect(() => {
    const selectionEnabled =
      activityEnabled && (isReminders || allowMixedSelection);
    if (!selectionEnabled) {
      selectReminder(null);
      return;
    }
    if (
      selectedReminderId !== null &&
      !pendingReminders.some((reminder) => reminder.id === selectedReminderId)
    ) {
      selectReminder(null);
      return;
    }
    if (!isReminders) return;
    if (viewportWidthPx === 0 || selectedReminder !== null) return;
    selectReminder(
      isNarrowHomeViewport ? null : (pendingReminders[0]?.id ?? null),
    );
  }, [
    activityEnabled,
    allowMixedSelection,
    isNarrowHomeViewport,
    isReminders,
    pendingReminders,
    selectedReminderId,
    selectedReminder,
    viewportWidthPx,
  ]);

  const drafts = useHomeDrafts({
    autoSelect: isDrafts,
    isNarrowHomeViewport,
    selectionEnabled: isDrafts || (activityEnabled && allowMixedSelection),
    viewportWidthPx,
  });

  return {
    drafts,
    dueReminderCount,
    pendingReminders,
    reminders: {
      selectedId: selectedReminderId,
      selectedItem: selectedReminder,
      select: selectReminder,
    },
  };
}
