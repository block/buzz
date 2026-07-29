import type * as React from "react";

import type { ChannelRef } from "@/features/dev-mode/lib/channelRefs";
import { DevLink } from "@/features/dev-mode/ui/DevLink";

/**
 * Conservative keyword highlighting for developer-mode transcripts. Message
 * text is tokenized into React spans — never HTML — so arbitrary content
 * stays inert. Highlighted tokens: `inline code`, URLs, @mentions, and
 * #channel references to known channels.
 */

const TOKEN_RE = /(`[^`\n]+`|https?:\/\/[^\s<>"')\]]+|@[\w./-]+|#[\w./-]+)/g;

/** Same boundary the composer's mention-prefix check uses. */
const MENTION_BOUNDARY_RE = /[\s,.;:!?)\]]/;

/** A name the message explicitly mentions (from its p tags) and its chat color. */
export type MentionStyle = { name: string; color: string };

/** Click handling for `#channel-name` references to known channels. */
export type ChannelRefOptions = {
  channels: ChannelRef[];
  onOpen: (channelId: string) => void;
};

/**
 * Longest known channel name (case-insensitive) starting right after the `#`
 * at `hashIndex`, with the same word-boundary rule mentions use.
 */
function matchKnownChannel(
  content: string,
  hashIndex: number,
  channels: ChannelRef[],
): ChannelRef | null {
  let best: ChannelRef | null = null;
  for (const channel of channels) {
    if (best && channel.name.length <= best.name.length) continue;
    const end = hashIndex + 1 + channel.name.length;
    const candidate = content.slice(hashIndex + 1, end);
    if (candidate.toLowerCase() !== channel.name.toLowerCase()) continue;
    const after = content[end];
    if (after !== undefined && !MENTION_BOUNDARY_RE.test(after)) continue;
    best = channel;
  }
  return best;
}

/**
 * Longest known name (case-insensitive) starting right after the `@` at
 * `atIndex`. Names may contain spaces/parens — e.g. "amp (local)" — which the
 * generic token regex cannot capture.
 */
function matchKnownMention(
  content: string,
  atIndex: number,
  mentions: MentionStyle[],
): MentionStyle | null {
  let best: MentionStyle | null = null;
  for (const mention of mentions) {
    if (best && mention.name.length <= best.name.length) continue;
    const end = atIndex + 1 + mention.name.length;
    const candidate = content.slice(atIndex + 1, end);
    if (candidate.toLowerCase() !== mention.name.toLowerCase()) continue;
    const after = content[end];
    if (after !== undefined && !MENTION_BOUNDARY_RE.test(after)) continue;
    best = mention;
  }
  return best;
}

export function renderHighlightedContent(
  content: string,
  mentions: MentionStyle[] = [],
  channelRefs?: ChannelRefOptions,
): React.ReactNode[] {
  const nodes: React.ReactNode[] = [];
  const re = new RegExp(TOKEN_RE.source, "g");
  let lastIndex = 0;
  let match = re.exec(content);
  while (match !== null) {
    const index = match.index;
    if (index > lastIndex) {
      nodes.push(content.slice(lastIndex, index));
    }
    const token = match[0];
    if (token.startsWith("`")) {
      nodes.push(
        <span
          key={`${index}-code`}
          className="rounded-none bg-muted/60 px-0.5 text-amber-500 dark:text-amber-300"
        >
          {token.slice(1, -1)}
        </span>,
      );
      lastIndex = index + token.length;
    } else if (token.startsWith("@")) {
      const known = matchKnownMention(content, index, mentions);
      if (known) {
        const end = index + 1 + known.name.length;
        // Matches the composer's mode pill: bordered box in the agent's color.
        nodes.push(
          <span
            key={`${index}-mention`}
            className="rounded-none border px-1 font-medium"
            style={{ color: known.color, borderColor: `${known.color}80` }}
          >
            {content.slice(index, end)}
          </span>,
        );
        lastIndex = end;
        re.lastIndex = end;
      } else {
        nodes.push(
          <span key={`${index}-mention`} className="font-medium text-sky-500">
            {token}
          </span>,
        );
        lastIndex = index + token.length;
      }
    } else if (token.startsWith("#")) {
      const known = channelRefs
        ? matchKnownChannel(content, index, channelRefs.channels)
        : null;
      if (known && channelRefs) {
        const end = index + 1 + known.name.length;
        const onOpen = channelRefs.onOpen;
        // mousedown is prevented so a click never pulls focus off the
        // composer — the same rule message clicks follow.
        nodes.push(
          <button
            key={`${index}-channel`}
            className="cursor-pointer font-medium text-sky-500 hover:underline"
            onClick={() => onOpen(known.id)}
            onMouseDown={(event) => event.preventDefault()}
            type="button"
          >
            {content.slice(index, end)}
          </button>,
        );
        lastIndex = end;
        re.lastIndex = end;
      } else {
        // `#word` that is not a known channel is ordinary prose.
        nodes.push(token);
        lastIndex = index + token.length;
      }
    } else {
      // Sentence punctuation after a URL belongs to the prose, not the href.
      const href = token.replace(/[.,;:!?]+$/, "");
      nodes.push(<DevLink href={href} key={`${index}-url`} />);
      if (href.length < token.length) {
        nodes.push(token.slice(href.length));
      }
      lastIndex = index + token.length;
    }
    match = re.exec(content);
  }
  if (lastIndex < content.length) {
    nodes.push(content.slice(lastIndex));
  }
  return nodes;
}
