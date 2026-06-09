/**
 * Remark plugin that detects #channel-name patterns in text nodes and wraps them
 * in custom HAST `channel-link` elements for styled rendering via react-markdown.
 *
 * Known channel names are matched longest-first to avoid partial matches. When
 * no known names are provided, falls back to `#\S+` so that channel links still
 * render while the channel list is loading asynchronously.
 */

import { createRemarkPrefixPlugin } from "./createRemarkPrefixPlugin";
import { buildPrefixPattern } from "./mentionPattern";

type RemarkChannelLinksOptions = {
  channelNames?: string[];
};

export default function remarkChannelLinks(
  options?: RemarkChannelLinksOptions,
) {
  const channelPattern = buildPrefixPattern("#", options?.channelNames ?? [], {
    fallbackToGeneric: true,
  });

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
