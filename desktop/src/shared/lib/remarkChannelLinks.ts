/**
 * Remark plugin that detects #channel-name patterns in text nodes and wraps them
 * in custom HAST `channel-link` elements for styled rendering via react-markdown.
 *
 * Known channel names are matched longest-first to avoid partial matches. A
 * generic channel token also matches independently of the reader's membership
 * or directory loading state; resolution determines whether the chip opens.
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
    genericTokenPattern: "[A-Za-z0-9_][A-Za-z0-9_-]*",
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
