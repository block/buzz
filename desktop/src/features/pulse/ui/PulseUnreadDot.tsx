import { useAppShell } from "@/app/AppShellContext";

/** Actual unread projections, never inferred from recency or channel activity. */
export function usePulseUnreadChannels() {
  const { topLevelUnreadChannelIds, unreadThreadChannelIds } = useAppShell();
  return (channelId: string) =>
    topLevelUnreadChannelIds.has(channelId) ||
    unreadThreadChannelIds.has(channelId);
}

export const PULSE_STATUS_DOT_CLASS = "h-[6px] w-[6px] shrink-0 rounded-full";

/** The containing row owns the accessible unread description. */
export function PulseUnreadDot() {
  return (
    <span
      aria-hidden
      data-testid="pulse-unread-dot"
      className={`ml-auto bg-blue-600 dark:bg-blue-400 ${PULSE_STATUS_DOT_CLASS}`}
    />
  );
}
