import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import type { TimelineMessage } from "@/features/messages/types";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";

/**
 * The newest main-timeline message the react-to-latest shortcut can target:
 * pending sends have no delivered event to react to, and system rows
 * (join/leave/channel-created) are not what "react to the latest message"
 * means conversationally. Entries arrive in render order (oldest first).
 */
export function findLatestReactableMessage(
  entries: readonly MainTimelineEntry[],
): TimelineMessage | null {
  for (let index = entries.length - 1; index >= 0; index -= 1) {
    const message = entries[index]?.message;
    if (!message || message.pending || message.kind === KIND_SYSTEM_MESSAGE) {
      continue;
    }
    return message;
  }
  return null;
}
