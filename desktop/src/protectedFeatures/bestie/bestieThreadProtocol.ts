/** Markers + prompts for Bestie thread summarize turns (top-level DM). */

export const BESTIE_THREAD_SUMMARIZE_MARKER = "[Bestie thread summarize]";
export const BESTIE_THREAD_SUMMARIZE_EVENT = "buzz:bestie-thread-summarize";

export function bestieThreadId(channelId: string, rootEventId: string): string {
  return `${channelId}:${rootEventId}`;
}

export function formatBestieThreadSummarizePrompt(thread: {
  channelId: string;
  channelName: string | null;
  preview: string;
  rootEventId: string;
  authorName?: string | null;
}): string {
  const where =
    thread.channelName?.trim() ||
    `channel ${thread.channelId.slice(0, 8)}…`;
  const who = thread.authorName?.trim() ? ` (from ${thread.authorName})` : "";
  const trackingId = bestieThreadId(thread.channelId, thread.rootEventId);
  return `${BESTIE_THREAD_SUMMARIZE_MARKER}

Summarize this thread Assistant is tracking in ${where}.
Tracking id: ${trackingId}
Root event: ${thread.rootEventId}
Channel id: ${thread.channelId}

Use \`buzz messages thread --channel ${thread.channelId} --event ${thread.rootEventId}\` if you need full history (channel membership or open visibility required on the relay).

Structure the reply as:
- **Covered** — bullet list of what was discussed / decided
- **Outstanding** — brief open items still to work
- **Expected of you** — bullets of anything asking Assistant / you to do next
- **Mentions of you** — anything that @mentions or clearly addresses you

Starter context${who}:
${thread.preview.trim() || "(no preview)"}`;
}

/** Parse composite thread id from a summarize prompt body. */
export function parseBestieThreadIdFromSummarizePrompt(
  content: string,
): string | null {
  const match = content.match(/^Tracking id:\s*(.+?)\s*$/m);
  const id = match?.[1]?.trim() ?? "";
  if (!id || !id.includes(":")) return null;
  return id;
}
