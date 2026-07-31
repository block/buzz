import {
  getActivityHeadline,
  isMeaningfulItem,
  isSpineItem,
} from "@/features/agents/ui/agentSessionTranscriptPresentation";
import type { TranscriptItem } from "@/features/agents/ui/agentSessionTypes";

/**
 * Latest agent-activity headline for one channel, or null when the observer
 * transcript has no channel-scoped signal yet (turn just started, or the
 * agent runs without an observer stream).
 *
 * Reuses the standard UI's two-tier scan (see BotActivityBar): spine items —
 * tools, thoughts, assistant messages — headline over metadata reads, which
 * only surface when no spine work exists yet. Unlike the standard bar this
 * keeps only the newest headline; the dev-mode status line is a single quiet
 * line, not a rotating carousel. The user's own prompt echo is excluded —
 * the line reports what the agent is doing, not what was asked.
 */
export function selectLatestActivityHeadline(
  transcript: readonly TranscriptItem[],
  channelId: string,
): string | null {
  const scoped = transcript.filter(
    (item) =>
      item.channelId === channelId &&
      !(item.type === "message" && item.role === "user"),
  );
  const passFilter = scoped.some(isSpineItem) ? isSpineItem : isMeaningfulItem;

  for (let i = scoped.length - 1; i >= 0; i--) {
    const item = scoped[i];
    if (item === undefined || !passFilter(item)) {
      continue;
    }
    const headline = getActivityHeadline(item);
    if (headline) {
      return headline;
    }
  }

  return null;
}
