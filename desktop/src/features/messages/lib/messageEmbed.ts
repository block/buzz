import type { Channel, RelayEvent } from "@/shared/api/types";

import { parseMessageLink, type ParsedMessageLink } from "./messageLink";

const MESSAGE_LINK_PATTERN = /(?:buzz):\/\/message\?[^\s<>"')\]]+/g;
const TRAILING_PUNCTUATION_PATTERN = /[.,;:!?]+$/;
const MAX_MESSAGE_EMBEDS = 4;

export type MessageEmbedLink = ParsedMessageLink & { href: string };

function collectCodeRanges(
  content: string,
): Array<{ start: number; end: number }> {
  const ranges: Array<{ start: number; end: number }> = [];
  for (const match of content.matchAll(/```[\s\S]*?```|~~~[\s\S]*?~~~/g)) {
    ranges.push({
      start: match.index ?? 0,
      end: (match.index ?? 0) + match[0].length,
    });
  }
  for (const match of content.matchAll(/`[^`\n]*`/g)) {
    ranges.push({
      start: match.index ?? 0,
      end: (match.index ?? 0) + match[0].length,
    });
  }
  return ranges;
}

function isInsideRange(
  index: number,
  ranges: Array<{ start: number; end: number }>,
) {
  return ranges.some((range) => index >= range.start && index < range.end);
}

/** Extract only bare message permalinks; authored markdown labels stay labels. */
export function extractMessageEmbedLinks(content: string): MessageEmbedLink[] {
  const codeRanges = collectCodeRanges(content);
  const links: MessageEmbedLink[] = [];
  const seen = new Set<string>();

  for (const match of content.matchAll(MESSAGE_LINK_PATTERN)) {
    const index = match.index ?? 0;
    if (isInsideRange(index, codeRanges)) continue;
    // `[label](buzz://…)` is intentionally labeled and must not unfurl.
    if (content.slice(Math.max(0, index - 2), index) === "](") continue;

    const href = match[0].replace(TRAILING_PUNCTUATION_PATTERN, "");
    if (seen.has(href)) continue;
    const parsed = parseMessageLink(href);
    if (!parsed.ok) continue;

    seen.add(href);
    links.push({ href, ...parsed.value });
    if (links.length === MAX_MESSAGE_EMBEDS) break;
  }

  return links;
}

export function canReadMessageEmbedSource(
  channel: Pick<Channel, "isMember" | "visibility"> | undefined,
): boolean {
  return (
    channel !== undefined && (channel.isMember || channel.visibility === "open")
  );
}

export function isMatchingMessageEmbedEvent(
  event: RelayEvent,
  link: ParsedMessageLink,
): boolean {
  const eventChannelId = event.tags.find((tag) => tag[0] === "h")?.[1];
  return event.id === link.messageId && eventChannelId === link.channelId;
}

export function messageEmbedExcerpt(content: string): string {
  return content.replace(/\s+/g, " ").trim();
}
