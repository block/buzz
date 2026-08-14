import { createRemarkPrefixPlugin } from "../../../shared/lib/createRemarkPrefixPlugin.ts";

import { isLongFormNaddr } from "./nostrAddress.ts";

const NADDR_URL_PATTERN = /nostr:naddr1[^\s<>"')\]]+/g;
const TRAILING_PUNCTUATION_PATTERN = /[.,;:!?]+$/;

function trimNaddrLinkMatch(matchText: string) {
  const value = matchText.replace(TRAILING_PUNCTUATION_PATTERN, "");
  return { value, trailing: matchText.slice(value.length) };
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
