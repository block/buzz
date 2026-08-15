/**
 * Content-assembly for the "Forward message" feature.
 *
 * Forwarding is built entirely out of existing, already-active primitives —
 * see the module doc in `ForwardMessageDialog.tsx` for the full picture. This
 * file owns the pure (non-network) half: given one or more source messages,
 * produce the single combined markdown body and the union of their imeta
 * attachment tags that get published as ONE new message per destination.
 *
 * Each source message is rendered as a blockquote block (`>` prefix, which
 * the existing markdown renderer already supports — no new syntax needed):
 *
 *   > **Forwarded from Alice**
 *   > message body line 1
 *   > message body line 2
 *
 * Multiple forwarded messages are stacked in original chronological order,
 * separated by a blank line, each with its own attribution line.
 */

import type { TimelineMessage } from "@/features/messages/types";
import {
  buildImetaTags,
  formatImetaMediaLine,
  type ImetaMedia,
} from "@/features/messages/lib/imetaMediaMarkdown";
import {
  parseImetaTags,
  type ParsedImetaEntry,
} from "@/shared/ui/markdown/parseImeta";

/**
 * Strip the trailing `![image|video](url)` / `[filename](url)` attachment
 * lines that `imetaMediaMarkdown` appends to a message body — those lines
 * reference the ORIGINAL message's imeta tags positionally, which is only
 * meaningful in that message. The forwarded copy re-appends its own
 * attachment lines (via `formatImetaMediaLine`) right after quoting it, so
 * duplicating the raw lines here would either dangle (blockquoted but with no
 * matching imeta tag order) or double up when combined with the extracted
 * imeta below. This is a best-effort trim: any line that looks like a
 * standalone image/video/file link is dropped from the quoted text and
 * re-added explicitly from the parsed imeta entries instead.
 */
function stripAttachmentLines(body: string, urls: ReadonlySet<string>): string {
  if (urls.size === 0) return body;
  const lines = body.split("\n");
  const kept = lines.filter((line) => {
    const trimmed = line.trim();
    const match = trimmed.match(/^!?\[[^\]]*\]\(([^)\s]+)\)$/);
    return !(match && urls.has(match[1]));
  });
  return kept.join("\n").replace(/\s+$/, "");
}

function imetaEntryToMedia(entry: ParsedImetaEntry): ImetaMedia {
  return {
    url: entry.url,
    type: entry.m || "image/jpeg",
    sha256: entry.x ?? "",
    size: entry.size ?? 0,
    uploaded: 0,
    ...(entry.dim ? { dim: entry.dim } : {}),
    ...(entry.blurhash ? { blurhash: entry.blurhash } : {}),
    ...(entry.thumb ? { thumb: entry.thumb } : {}),
    ...(entry.duration != null ? { duration: entry.duration } : {}),
    ...(entry.image ? { image: entry.image } : {}),
    ...(entry.filename ? { filename: entry.filename } : {}),
  };
}

/** Blockquote every non-empty line of `text` (blank lines stay blank so the
 *  blockquote doesn't visually merge with any following paragraph). */
function quoteLines(text: string): string {
  return text
    .split("\n")
    .map((line) => (line.length === 0 ? ">" : `> ${line}`))
    .join("\n");
}

export function forwardedMessageAuthorLabel(message: TimelineMessage): string {
  return message.author || "Unknown";
}

/**
 * Build the combined body + imeta tag set for forwarding `messages` (in
 * chronological order) as a single new message. Safe to call with a single
 * message (the single-message "Forward" action) or many (multi-select
 * bundling) — the output shape is identical either way, per the product
 * requirement that a multi-forward is still exactly one message per
 * destination.
 */
export function buildForwardedContent(messages: readonly TimelineMessage[]): {
  content: string;
  mediaTags: string[][] | undefined;
} {
  const header =
    messages.length <= 1 ? "Forwarded message" : `Forwarded ${messages.length} messages`;

  const allMedia: ImetaMedia[] = [];
  const seenUrls = new Set<string>();

  const blocks = messages.map((message) => {
    const imetaByUrl = message.tags ? parseImetaTags(message.tags) : undefined;
    const mediaForMessage: ImetaMedia[] = [];
    if (imetaByUrl) {
      for (const entry of imetaByUrl.values()) {
        const media = imetaEntryToMedia(entry);
        mediaForMessage.push(media);
        if (!seenUrls.has(media.url)) {
          seenUrls.add(media.url);
          allMedia.push(media);
        }
      }
    }

    const urls = new Set(mediaForMessage.map((media) => media.url));
    const bodyWithoutAttachmentLines = stripAttachmentLines(
      message.body ?? "",
      urls,
    );

    let quoted = `> **Forwarded from ${forwardedMessageAuthorLabel(message)}**`;
    if (bodyWithoutAttachmentLines.trim().length > 0) {
      quoted += `\n${quoteLines(bodyWithoutAttachmentLines)}`;
    }
    for (const media of mediaForMessage) {
      // Attachment lines ride outside the blockquote (a leading blank line
      // from formatImetaMediaLine already separates them) so the renderer's
      // image/video/file-card upgrade — which matches on the raw markdown
      // line, not blockquoted text — still fires.
      quoted += formatImetaMediaLine(media);
    }
    return quoted;
  });

  const content = `${header}\n\n${blocks.join("\n\n")}`;
  const mediaTags = allMedia.length > 0 ? buildImetaTags(allMedia) : undefined;

  return { content, mediaTags };
}
