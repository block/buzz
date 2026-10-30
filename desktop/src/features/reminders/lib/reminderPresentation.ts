import {
  formatNotificationTitle,
  resolveNotificationChannelLabel,
  truncateNotificationBody,
} from "@/features/notifications/lib/notificationFormat";
import type { Reminder } from "@/features/reminders/lib/reminderTypes";

/**
 * The headline text for a reminder: the target message preview when there is
 * one, else the note of a standalone reminder.
 */
export function reminderPreviewText(reminder: Reminder): string {
  return (
    reminder.content.target?.preview || reminder.content.note || "Reminder"
  );
}

/** Event IDs of the messages that have a pending message reminder. */
export function activeReminderEventIds(
  reminders: readonly Reminder[],
): Set<string> {
  const ids = new Set<string>();
  for (const reminder of reminders) {
    const eventId = reminder.content.target?.eventId;
    if (reminder.content.status === "pending" && eventId) ids.add(eventId);
  }
  return ids;
}

/**
 * The desktop notification for newly due reminders. One reminder shows its
 * message preview or note, with its channel when it has a target; several are
 * coalesced into a count.
 */
export function describeDueReminders(
  due: readonly Reminder[],
  channels: ReadonlyArray<{ id: string; name?: string | null }>,
): { title: string; body: string } {
  const [only] = due.length === 1 ? due : [];
  const channelLabel = only
    ? resolveNotificationChannelLabel(
        only.content.target?.channelId ?? null,
        channels,
      )
    : null;
  const body = only
    ? truncateNotificationBody(
        only.content.target?.preview ?? only.content.note ?? "",
        "A reminder is waiting",
      )
    : `${due.length} reminders are due`;
  return {
    title: formatNotificationTitle({ prefix: "Reminder due", channelLabel }),
    body,
  };
}
