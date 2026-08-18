/**
 * Remark plugin that detects @mention patterns in text nodes and wraps them
 * in custom HAST `mention` elements for styled rendering via react-markdown.
 *
 * Only p-tagged member names are highlighted — multi-word display names
 * (e.g. "John Doe") are matched longest-first to avoid partial matches.
 * When no known names are provided, nothing is highlighted.
 */

import { createRemarkPrefixPlugin } from "./createRemarkPrefixPlugin";
import { buildMentionPattern } from "./mentionPattern";

type RemarkMentionsOptions = {
  mentionNames?: string[];
};

export default function remarkMentions(options?: RemarkMentionsOptions) {
  const mentionPattern = buildMentionPattern(options?.mentionNames ?? []);

  return createRemarkPrefixPlugin(mentionPattern, (matchText) => ({
    type: "mention",
    value: matchText,
    data: {
      hName: "mention",
      hChildren: [{ type: "text", value: matchText }],
    },
  }));
}
