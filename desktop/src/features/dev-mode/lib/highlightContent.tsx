import type * as React from "react";

import { DevLink } from "@/features/dev-mode/ui/DevLink";

/**
 * Conservative keyword highlighting for developer-mode transcripts. Message
 * text is tokenized into React spans — never HTML — so arbitrary content
 * stays inert. Highlighted tokens: `inline code`, URLs, and @mentions.
 */

const TOKEN_RE = /(`[^`\n]+`|https?:\/\/[^\s<>"')\]]+|@[\w./-]+)/g;

/** Same boundary the composer's mention-prefix check uses. */
const MENTION_BOUNDARY_RE = /[\s,.;:!?)\]]/;

/** A name the message explicitly mentions (from its p tags) and its chat color. */
export type MentionStyle = { name: string; color: string };

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
