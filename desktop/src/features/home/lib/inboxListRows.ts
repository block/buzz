import { withoutDuplicatedChannels } from "@/features/home/lib/channelUnreadRows.mjs";
import type { ChannelUnreadRow } from "@/features/home/lib/channelUnreadRows.mjs";
import type { InboxItem } from "@/features/home/lib/inbox";
import type { Reminder } from "@/features/reminders/lib/reminderTypes";

export type InboxListRow =
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
  | ChannelUnreadRow;

export function buildInboxListRows({
  channelRows,
  items,
  reminders,
}: {
  // Channel-level unread rows, derived from the sidebar's own unread
  // projections. Only supplied for the All filter — the narrower filters are
  // definitionally about feed categories, so a generic "4 new" row has no
  // meaning under Mentions or Drafts.
  channelRows?: readonly ChannelUnreadRow[];
  items: readonly InboxItem[];
  reminders: readonly Reminder[];
}): InboxListRow[] {
  const consumedReminderIds = new Set<string>();
  const inboxRows = items.map((item): InboxListRow => {
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

  // A channel that already has a feed row must not also get a generic one:
  // the feed row names what actually happened, so it wins and the channel row
  // is dropped. See withoutDuplicatedChannels for the cost of that choice.
  const occupiedChannelIds = new Set<string>();
  for (const item of items) {
    const channelId = item.item.channelId;
    if (channelId) occupiedChannelIds.add(channelId);
  }

  return [
    ...inboxRows,
    ...withoutDuplicatedChannels(channelRows ?? [], occupiedChannelIds),
    ...reminders
      .filter(
        (reminder) =>
          reminder.content.status === "pending" &&
          !consumedReminderIds.has(reminder.id),
      )
      .map(
        (reminder): InboxListRow => ({
          key: `reminder:${reminder.id}`,
          kind: "reminder",
          reminder,
          sortAt: reminder.notBefore ?? reminder.createdAt,
        }),
      ),
  ].sort((left, right) => right.sortAt - left.sortAt);
}
