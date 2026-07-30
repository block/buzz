import { createRemarkPrefixPlugin } from "../../../shared/lib/createRemarkPrefixPlugin.ts";

import { isLongFormNaddr } from "./nostrAddress.ts";

const NADDR_URL_PATTERN = /nostr:naddr1[^\s<>"')\]]+/g;
const TRAILING_PUNCTUATION_PATTERN = /[.,;:!?]+$/;

function trimNaddrLinkMatch(matchText: string) {
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

export default function remarkNaddrLinks() {
  return createRemarkPrefixPlugin(NADDR_URL_PATTERN, (matchText) => {
    const { value, trailing } = trimNaddrLinkMatch(matchText);
    if (!isLongFormNaddr(value)) {
      return { type: "text", value: matchText };
    }

    return {
      node: {
        type: "link",
        url: value,
        children: [{ type: "text", value }],
      },
      trailing,
    };
  });
}
