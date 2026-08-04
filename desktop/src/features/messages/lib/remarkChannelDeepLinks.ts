/** Detect bare `buzz://channel/<uuid>` URLs in markdown text nodes. */
import { createRemarkPrefixPlugin } from "../../../shared/lib/createRemarkPrefixPlugin.ts";

const CHANNEL_URL_PATTERN = /buzz:\/\/channel\/[^\s<>"')\]]+/g;
const TRAILING_PUNCTUATION_PATTERN = /[.,;:!?]+$/;

export default function remarkChannelDeepLinks() {
  return createRemarkPrefixPlugin(CHANNEL_URL_PATTERN, (matchText) => {
    const value = matchText.replace(TRAILING_PUNCTUATION_PATTERN, "");
    return {
      node: {
        type: "channel-deep-link",
        value,
        data: {
          hName: "channel-deep-link",
          hChildren: [{ type: "text", value }],
        },
      },
      trailing: matchText.slice(value.length),
    };
  });
}
