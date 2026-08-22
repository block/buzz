import { TranscriptTimestamp } from "./activityRenderClasses/TranscriptTimestamp";

/**
 * Opt-in per-row timestamp, anchored bottom-left under the row content and
 * styled to match the chat/transcript timestamps.
 *
 * Extracted from `AgentSessionTranscriptList` so tool-run group chrome can
 * render the same timestamp without importing the list module (which imports
 * the group card, and would form a cycle).
 */
export function TranscriptRowTimestamp({
  messageLink = null,
  timestamp,
}: {
  messageLink?: { channelId: string; messageId: string } | null;
  timestamp: string;
}) {
  return (
    <div
      className="mt-0.5 flex justify-start"
      data-testid="transcript-row-timestamp"
    >
      <TranscriptTimestamp messageLink={messageLink} timestamp={timestamp} />
    </div>
  );
}
