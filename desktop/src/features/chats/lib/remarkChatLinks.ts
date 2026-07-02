/**
 * Remark plugin that detects bare `buzz://chat?…` URLs in text nodes and
 * replaces each with a custom `chat-link` element. The renderer turns the bare
 * link into a native chat card, while explicitly labeled markdown links remain
 * inline labels.
 */
import { createRemarkPrefixPlugin } from "../../../shared/lib/createRemarkPrefixPlugin.ts";

const CHAT_URL_PATTERN = /buzz:\/\/chat\?[^\s<>"')\]]+/g;
const TRAILING_PUNCTUATION_PATTERN = /[.,;:!?]+$/;

function trimChatLinkMatch(matchText: string) {
  let value = matchText.replace(TRAILING_PUNCTUATION_PATTERN, "");
  while (/[)\]]$/.test(value) && isUnmatchedClosing(value)) {
    value = value.slice(0, -1).replace(TRAILING_PUNCTUATION_PATTERN, "");
  }
  return { value, trailing: matchText.slice(value.length) };
}

function isUnmatchedClosing(value: string): boolean {
  const closing = value[value.length - 1];
  const opening = closing === ")" ? "(" : "[";
  return value.split(closing).length > value.split(opening).length;
}

export default function remarkChatLinks() {
  return createRemarkPrefixPlugin(CHAT_URL_PATTERN, (matchText) => {
    const { value, trailing } = trimChatLinkMatch(matchText);

    return {
      node: {
        type: "chat-link",
        value,
        data: {
          hName: "chat-link",
          hChildren: [{ type: "text", value }],
        },
      },
      trailing,
    };
  });
}
