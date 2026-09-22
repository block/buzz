import { i18n } from "@/i18n";

/**
 * Shared reminder time presets — the single source of truth for both the
 * create dialog and the snooze dropdown. Each preset returns a Unix timestamp
 * (seconds) strictly in the future.
 */
export type TimePreset = {
  /**
   * Resolved on every read: `i18n` boots after this module is imported, so a
   * module-top-level `i18n.t(...)` would freeze `undefined` into the list.
   */
  label: string;
  getTimestamp: () => number;
};

function nowSeconds(): number {
  return Math.floor(Date.now() / 1_000);
}

/**
 * Next occurrence of `dayOffset` days from now at 9am local time. If that
 * instant is already past (e.g. it is after 9am and offset is 0), roll to the
 * following day so the result is always in the future.
 */
function nextDayAt9am(dayOffset: number): number {
  const now = new Date();
  const target = new Date(now);
  target.setDate(target.getDate() + dayOffset);
  target.setHours(9, 0, 0, 0);
  if (target.getTime() <= now.getTime()) {
    target.setDate(target.getDate() + 1);
  }
  return Math.floor(target.getTime() / 1_000);
}

export const TIME_PRESETS: TimePreset[] = [
  {
    get label() {
      return i18n.t("reminders.preset.in-30-minutes");
    },
    getTimestamp: () => nowSeconds() + 30 * 60,
  },
  {
    get label() {
      return i18n.t("reminders.preset.in-1-hour");
    },
    getTimestamp: () => nowSeconds() + 60 * 60,
  },
  {
    get label() {
      return i18n.t("reminders.preset.in-3-hours");
    },
    getTimestamp: () => nowSeconds() + 3 * 60 * 60,
  },
  {
    get label() {
      return i18n.t("reminders.preset.tomorrow-9am");
    },
    getTimestamp: () => nextDayAt9am(1),
  },
  {
    get label() {
      return i18n.t("reminders.preset.next-monday-9am");
    },
    getTimestamp: () => {
      const daysUntilMonday = (8 - new Date().getDay()) % 7 || 7;
      return nextDayAt9am(daysUntilMonday);
    },
  },
];

/** Today as `YYYY-MM-DD` in local time, for the custom date input `min`. */
export function todayDateString(): string {
  const now = new Date();
  const year = now.getFullYear();
  const month = String(now.getMonth() + 1).padStart(2, "0");
  const day = String(now.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

/**
 * Parse a `YYYY-MM-DD` + `HH:MM` pair into a future Unix timestamp (seconds),
 * or null if the inputs are malformed or not strictly in the future. The shared
 * guard for both create and snooze custom surfaces: the native time input has
 * no `min`, so a past time would otherwise fire immediately.
 */
export function parseCustomDateTime(date: string, time: string): number | null {
  if (!date || !time) return null;
  const timestamp = Math.floor(new Date(`${date}T${time}`).getTime() / 1_000);
  if (Number.isNaN(timestamp)) return null;
  if (timestamp <= nowSeconds()) return null;
  return timestamp;
}
