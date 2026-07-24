import type { InboxItem } from "@/features/home/lib/inbox";
import type { DraftViewItem } from "@/features/messages/ui/DraftsPanel";
import type { Reminder } from "@/features/reminders/lib/reminderTypes";

export type ActivityListRow =
  | {
      key: string;
      kind: "inbox";
      item: InboxItem;
      dueReminder?: Reminder;
      sortAt: number;
    }
  | {
      key: string;
      kind: "reminder";
      reminder: Reminder;
      sortAt: number;
    }
  | {
      key: string;
      kind: "draft";
      item: DraftViewItem;
      sortAt: number;
    };

function draftActivityAt(item: DraftViewItem): number {
  for (const value of [
    item.entry.draft.updatedAt,
    item.entry.draft.createdAt,
  ]) {
    const timestamp = Date.parse(value);
    if (Number.isFinite(timestamp)) return timestamp / 1_000;
  }
  return 0;
}

export function buildActivityListRows({
  drafts,
  items,
  reminders,
}: {
  drafts: readonly DraftViewItem[];
  items: readonly InboxItem[];
  reminders: readonly Reminder[];
}): ActivityListRow[] {
  const consumedReminderIds = new Set<string>();
  const inboxRows = items.map((item): ActivityListRow => {
    const eventIds = new Set([
      item.id,
      item.item.id,
      ...item.groupItems.map((groupItem) => groupItem.id),
    ]);
    const matchingReminders = reminders
      .filter(
        (reminder) =>
          reminder.content.status === "pending" &&
          Boolean(
            reminder.content.target?.eventId &&
              eventIds.has(reminder.content.target.eventId),
          ),
      )
      .sort(
        (left, right) =>
          (right.notBefore ?? right.createdAt) -
          (left.notBefore ?? left.createdAt),
      );
    const dueReminder = matchingReminders[0];

    for (const reminder of matchingReminders) {
      consumedReminderIds.add(reminder.id);
    }

    return {
      key: `inbox:${item.conversationId}`,
      kind: "inbox",
      item,
      dueReminder,
      sortAt: Math.max(
        item.latestActivityAt,
        dueReminder?.notBefore ?? dueReminder?.createdAt ?? 0,
      ),
    };
  });

  return [
    ...inboxRows,
    ...reminders
      .filter(
        (reminder) =>
          reminder.content.status === "pending" &&
          !consumedReminderIds.has(reminder.id),
      )
      .map(
        (reminder): ActivityListRow => ({
          key: `reminder:${reminder.id}`,
          kind: "reminder",
          reminder,
          sortAt: reminder.notBefore ?? reminder.createdAt,
        }),
      ),
    ...drafts
      .filter((item) => item.rootStatus !== "deleted")
      .map(
        (item): ActivityListRow => ({
          key: `draft:${item.entry.key}`,
          kind: "draft",
          item,
          sortAt: draftActivityAt(item),
        }),
      ),
  ].sort((left, right) => right.sortAt - left.sortAt);
}
