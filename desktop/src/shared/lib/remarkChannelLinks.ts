/**
 * Remark plugin that detects #channel-name patterns in text nodes and wraps them
 * in custom HAST `channel-link` elements for styled rendering via react-markdown.
 *
 * Only known channel names are highlighted — multi-word names (e.g. "my channel")
 * are matched longest-first to avoid partial matches. When no known names are
 * provided, nothing is highlighted.
 */

import { createRemarkPrefixPlugin } from "./createRemarkPrefixPlugin";
import { buildPrefixPattern } from "./mentionPattern";

type RemarkChannelLinksOptions = {
  channelNames?: string[];
};

export default function remarkChannelLinks(
  options?: RemarkChannelLinksOptions,
) {
  const channelPattern = buildPrefixPattern("#", options?.channelNames ?? []);

  return createRemarkPrefixPlugin(channelPattern, (matchText) => {
    const channelName = matchText.slice(1);
    return {
      type: "channel-link",
      value: matchText,
      data: {
        hName: "channel-link",
        hChildren: [{ type: "text", value: matchText }],
        channelName,
      },
    };
  });
}
