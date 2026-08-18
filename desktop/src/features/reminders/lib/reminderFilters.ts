import type { Reminder } from "@/features/reminders/lib/reminderTypes";

const nowSeconds = () => Math.floor(Date.now() / 1_000);

/** A pending reminder whose `notBefore` has arrived (`<= now`). */
export function isDue(reminder: Reminder, now: number): boolean {
  return (
    reminder.content.status === "pending" &&
    reminder.notBefore !== undefined &&
    reminder.notBefore <= now
  );
}

/**
 * Count pending reminders that are due or overdue — the single definition
 * shared by the inbox badge and the fire-on-due hook so the two surfaces can
 * never disagree on what "due" means.
 */
export function countDue(
  reminders: readonly Reminder[],
  now: number = nowSeconds(),
): number {
  return reminders.filter((r) => isDue(r, now)).length;
}

/**
 * Pending reminders that newly crossed `notBefore` since `watermark` — the
 * fire-on-due window. The strict `>` lower bound is deliberate: a reminder
 * already past at the seeded watermark (first launch) never fires a toast, so
 * history is not replayed. The upper bound (`<= now`) excludes future ones.
 */
export function dueSince(
  reminders: readonly Reminder[],
  watermark: number,
  now: number,
): Reminder[] {
  return reminders.filter(
    (r) =>
      r.content.status === "pending" &&
      r.notBefore !== undefined &&
      r.notBefore > watermark &&
      r.notBefore <= now,
  );
}

export type ReminderGroup = {
  label: string;
  reminders: Reminder[];
};

/**
 * Bucket pending reminders into Overdue/Today/Upcoming, and — when
 * `includeDone` is set — append a Completed group of done reminders. Cancelled
 * reminders are never surfaced.
 */
export function groupReminders(
  reminders: Reminder[],
  includeDone = false,
): ReminderGroup[] {
  const now = nowSeconds();
  const endOfToday = new Date();
  endOfToday.setHours(23, 59, 59, 999);
  const endOfTodaySecs = Math.floor(endOfToday.getTime() / 1_000);

  const overdue: Reminder[] = [];
  const today: Reminder[] = [];
  const upcoming: Reminder[] = [];
  const done: Reminder[] = [];

  for (const r of reminders) {
    if (r.content.status === "done") {
      if (includeDone) done.push(r);
      continue;
    }
    if (r.content.status !== "pending") continue;
    if (r.notBefore === undefined) continue;
    if (r.notBefore <= now) {
      overdue.push(r);
    } else if (r.notBefore <= endOfTodaySecs) {
      today.push(r);
    } else {
      upcoming.push(r);
    }
  }

  done.sort((a, b) => b.createdAt - a.createdAt);

  const groups: ReminderGroup[] = [];
  if (overdue.length > 0) groups.push({ label: "Overdue", reminders: overdue });
  if (today.length > 0) groups.push({ label: "Today", reminders: today });
  if (upcoming.length > 0)
    groups.push({ label: "Upcoming", reminders: upcoming });
  if (done.length > 0) groups.push({ label: "Completed", reminders: done });
  return groups;
}
