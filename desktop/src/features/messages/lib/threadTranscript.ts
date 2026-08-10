/**
 * Plain-text serialization of a message thread for "Copy thread".
 *
 * Turns the thread panel's loaded view (root message plus its visible
 * replies) into a readable transcript: one block per message — author,
 * full timestamp, then the message text — separated by blank lines, in
 * the panel's display order (root first). Media embeds the composer
 * appends to the body (`![image|video](url)` lines, see
 * `imetaMediaMarkdown.ts`) are replaced with short placeholders like
 * `[image: photo.png]` so the copied text stays readable outside the app.
 *
 * Pure functions — no React / DOM dependency — so the serialization rules
 * are covered by `threadTranscript.test.mjs`.
 */

import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import type { TimelineMessage } from "@/features/messages/types";
import { parseImetaTags } from "@/shared/ui/markdown/parseImeta";

import { formatFullDateTime } from "./dateFormatters";

/** The subset of `TimelineMessage` the transcript serializer reads. */
export type ThreadTranscriptMessage = Pick<
  TimelineMessage,
  "author" | "body" | "createdAt" | "edited" | "tags"
>;

/**
 * Matches the media embed lines the send path appends to message bodies:
 * `![image](url)` / `![video](url)`, optionally wrapped in `||spoiler||`
 * markers (see `formatImetaMediaLine`).
 */
const MEDIA_EMBED_RE = /(\|\|)?!\[(image|video)\]\((\S+?)\)(\|\|)?/g;

/** Matches plain markdown links, used to label imeta file attachments. */
const FILE_LINK_RE = /\[((?:\\.|[^\]\\])*)\]\((\S+?)\)/g;

function attachmentNamesByUrl(
  tags: ReadonlyArray<ReadonlyArray<string>> | undefined,
): Map<string, string | undefined> {
  const names = new Map<string, string | undefined>();
  if (!tags || tags.length === 0) {
    return names;
  }

  for (const [url, entry] of parseImetaTags(tags as string[][])) {
    names.set(url, entry.filename);
  }

  return names;
}

/**
 * Replace inline media embeds and imeta file links in a message body with
 * short placeholders — `[image: photo.png]`, `[video]`, `[file: notes.pdf]` —
 * using the event's imeta filenames when available.
 */
export function replaceMediaEmbedsWithPlaceholders(
  body: string,
  tags?: ReadonlyArray<ReadonlyArray<string>>,
): string {
  const names = attachmentNamesByUrl(tags);

  let text = body.replace(MEDIA_EMBED_RE, (_match, _open, kind, url) => {
    const filename = names.get(url);
    return filename ? `[${kind}: ${filename}]` : `[${kind}]`;
  });

  // Generic file attachments render as plain `[label](url)` links; collapse
  // the ones backed by an imeta tag so the copied text doesn't carry long
  // relay URLs. Other links (user-typed markdown) are left untouched.
  text = text.replace(FILE_LINK_RE, (match, label, url) => {
    if (!names.has(url)) {
      return match;
    }

    const name = label.replace(/\\([\\[\]])/g, "$1").trim();
    return name ? `[file: ${name}]` : "[file]";
  });

  return text;
}

function serializeMessage(message: ThreadTranscriptMessage): string {
  const editedSuffix = message.edited ? " (edited)" : "";
  const header = `${message.author} — ${formatFullDateTime(
    message.createdAt,
  )}${editedSuffix}`;
  const body = replaceMediaEmbedsWithPlaceholders(
    message.body,
    message.tags,
  ).trim();

  return body ? `${header}\n${body}` : header;
}

/**
 * Serialize thread messages into a plain-text transcript.
 *
 * Preserves the input order — callers pass messages in the thread panel's
 * display order (root first, then replies as rendered), which is the natural
 * reading order of the thread.
 */
export function serializeThreadMessages(
  messages: readonly ThreadTranscriptMessage[],
): string {
  return messages.map(serializeMessage).join("\n\n");
}

/**
 * Build the "Copy thread" transcript from the thread panel's loaded state:
 * the thread head followed by the visible reply entries.
 */
export function buildThreadTranscript(
  threadHead: TimelineMessage,
  entries: readonly MainTimelineEntry[],
): string {
  return serializeThreadMessages([
    threadHead,
    ...entries.map((entry) => entry.message),
  ]);
}
