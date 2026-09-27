import { isMeaningfulItem } from "@/features/agents/ui/agentSessionTranscriptPresentation";
import type { TranscriptItem } from "@/features/agents/ui/agentSessionTypes";

export type BestieActivitySessionBoundary = {
  firstMessageCreatedAt: number;
} | null;

/**
 * Activity rows suitable for Bestie (kept for a later Bestie DM feature;
 * the popover no longer renders activity).
 * Drops chat-message items (Bestie already renders DM bubbles), raw/suppressed
 * noise, and items outside the active Bestie DM channel.
 *
 * When `sessionBoundary` is null (Close Thread / no active session), returns
 * no rows — same session gate as Bestie chat messages. When set, keeps only
 * activity at or after the session's first message.
 */
export function filterBestieActivityItems(
  items: readonly TranscriptItem[],
  options: {
    channelId: string | null | undefined;
    sessionBoundary?: BestieActivitySessionBoundary;
  },
): TranscriptItem[] {
  if (options.sessionBoundary === null) return [];

  const channelId = options.channelId ?? null;
  const sessionStartedAtMs =
    options.sessionBoundary == null
      ? null
      : options.sessionBoundary.firstMessageCreatedAt * 1000;

  return items.filter((item) => {
    if (item.type === "message") return false;
    if (!isMeaningfulItem(item)) return false;
    if (item.renderClass === "raw-rail" || item.renderClass === "suppressed") {
      return false;
    }
    if (channelId && item.channelId && item.channelId !== channelId) {
      return false;
    }
    if (sessionStartedAtMs != null) {
      const itemAt = Date.parse(item.timestamp);
      if (!Number.isFinite(itemAt) || itemAt < sessionStartedAtMs) {
        return false;
      }
    }
    return true;
  });
}
