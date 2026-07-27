import { inOneHour, nextDayAt9am } from "@/features/reminders/lib/timePresets";
import type { ResolvedChannelNotifyState } from "@/features/notifications/lib/resolveChannelNotifyState";
import type { ChannelNotifyLevel } from "@/features/sidebar/lib/channelNotifyPrefsStorage";

/**
 * User-facing copy for the NIP-CN per-channel notification levels, shared by
 * the sidebar context menu and the channel sheet so the two surfaces can never
 * drift. Pure and React-free.
 */
export const CHANNEL_NOTIFY_LEVEL_OPTIONS: readonly {
  value: ChannelNotifyLevel;
  label: string;
  description: string;
}[] = [
  {
    value: "all",
    label: "All new posts",
    description: "Mark every new post unread.",
  },
  {
    value: "mentions",
    label: "Just mentions",
    description: "Only mentions and followed threads alert you.",
  },
  {
    value: "mute",
    label: "Mute and hide",
    description: "Hide the channel from the sidebar until it mentions you.",
  },
];

/**
 * Timed-mute presets. Both compute an absolute epoch on the setting device, so
 * the synced value stays correct on other devices and time zones.
 */
export const CHANNEL_MUTE_PRESETS: readonly {
  label: string;
  testId: string;
  getTimestamp: () => number;
}[] = [
  {
    label: "Mute for 1 hour",
    testId: "channel-notify-mute-1-hour",
    getTimestamp: inOneHour,
  },
  {
    label: "Mute until tomorrow",
    testId: "channel-notify-mute-tomorrow",
    getTimestamp: () => nextDayAt9am(1),
  },
];

/**
 * Sentence appended to the channel header description when the channel's
 * notification level is not the default, or null when it is.
 */
export function channelNotifyHeaderSuffix(
  state: ResolvedChannelNotifyState,
): string | null {
  if (state.level === "mute") return "Notifications: Muted";
  if (state.level === "mentions") return "Notifications: Just mentions";
  return null;
}

/**
 * "Muted until 9:04 AM" caption for a running timed mute. Includes the weekday
 * once the expiry falls outside the current local day.
 */
export function formatMuteUntil(
  untilSeconds: number,
  now: Date = new Date(),
): string {
  const until = new Date(untilSeconds * 1_000);
  const time = until.toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
  });
  if (until.toDateString() === now.toDateString()) return time;
  const weekday = until.toLocaleDateString(undefined, { weekday: "short" });
  return `${weekday} ${time}`;
}
