import type * as React from "react";

import { DevLink } from "@/features/dev-mode/ui/DevLink";

/**
 * Conservative keyword highlighting for developer-mode transcripts. Message
 * text is tokenized into React spans — never HTML — so arbitrary content
 * stays inert. Highlighted tokens: `inline code`, URLs, and @mentions.
 */

const TOKEN_RE = /(`[^`\n]+`|https?:\/\/[^\s<>"')\]]+|@[\w./-]+)/g;

export function renderHighlightedContent(content: string): React.ReactNode[] {
  const nodes: React.ReactNode[] = [];
  let lastIndex = 0;
  for (const match of content.matchAll(TOKEN_RE)) {
    const index = match.index ?? 0;
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
    } else if (token.startsWith("@")) {
      nodes.push(
        <span key={`${index}-mention`} className="font-medium text-sky-500">
          {token}
        </span>,
      );
    } else {
      // Sentence punctuation after a URL belongs to the prose, not the href.
      const href = token.replace(/[.,;:!?]+$/, "");
      nodes.push(<DevLink href={href} key={`${index}-url`} />);
      if (href.length < token.length) {
        nodes.push(token.slice(href.length));
      }
    }
    lastIndex = index + token.length;
  }
  if (lastIndex < content.length) {
    nodes.push(content.slice(lastIndex));
  }
  return nodes;
}
