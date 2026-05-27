/**
 * Remark plugin that detects bare `sprout://message?…` URLs in text nodes and
 * replaces each with a custom `message-link` HAST element. The `markdown.tsx`
 * components map renders that as an inline pill (channel name + click-to-open)
 * instead of the raw 100-char URL.
 *
 * Why this plugin exists: `remark-gfm`'s autolinker only covers `http(s)://`
 * and `www.`. Custom schemes like `sprout://` only reach the `<a>` component
 * override when the user wrote an explicit `[label](sprout://…)` link.
 *
 * Mirrors `remarkChannelLinks` / `remarkMentions` — same factory, same HAST
 * shape — so the rendering layer treats all three uniformly. Trailing
 * sentence punctuation (`. , ; : ! ?`) and unmatched closing brackets are
 * peeled off the match and emitted as plain text after the pill, so a URL
 * pasted at end-of-sentence still routes to the correct message id.
 */
// Explicit `.ts` extension lets this plugin be imported both by the Vite-built
// `markdown.tsx` and by `markdown.test.mjs` running under `node --test
// --experimental-strip-types`. `tsconfig.json` enables `allowImportingTsExtensions`.
import { createRemarkPrefixPlugin } from "../../../shared/lib/createRemarkPrefixPlugin.ts";

const MESSAGE_URL_PATTERN = /sprout:\/\/message\?[^\s<>"')\]]+/g;
const TRAILING_PUNCTUATION_PATTERN = /[.,;:!?]+$/;

function trimMessageLinkMatch(matchText: string) {
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

export default function remarkMessageLinks() {
  return createRemarkPrefixPlugin(MESSAGE_URL_PATTERN, (matchText) => {
    const { value, trailing } = trimMessageLinkMatch(matchText);

    return {
      node: {
        type: "message-link",
        value,
        data: {
          hName: "message-link",
          hChildren: [{ type: "text", value }],
        },
      },
      trailing,
    };
  });
}
