import type { InboxItem } from "@/features/home/lib/inbox";
import type { DraftViewItem } from "@/features/messages/ui/DraftsPanel";
import type { Reminder } from "@/features/reminders/lib/reminderTypes";

export type ActivityListRow =
  | {
      key: string;
      kind: "inbox";
      item: InboxItem;
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
  return [
    ...items.map(
      (item): ActivityListRow => ({
        key: `inbox:${item.id}`,
        kind: "inbox",
        item,
        sortAt: item.latestActivityAt,
      }),
    ),
    ...reminders
      .filter((reminder) => reminder.content.status === "pending")
      .map(
        (reminder): ActivityListRow => ({
          key: `reminder:${reminder.id}`,
          kind: "reminder",
          reminder,
          sortAt: reminder.createdAt,
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
