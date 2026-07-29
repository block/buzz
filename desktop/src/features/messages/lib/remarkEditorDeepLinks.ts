/**
 * Remark plugin that detects bare `cursor://…` and `vscode://…` URLs in text
 * nodes and replaces each with a standard mdast `link` so the markdown `a`
 * renderer (and OS opener) can open them.
 *
 * Why this plugin exists: `remark-gfm`'s autolinker only covers `http(s)://`
 * and `www.`. Editor deep links only reach the `<a>` override when written as
 * an explicit `[label](cursor://…)` markdown link — or when this plugin
 * promotes a bare URL into a link node.
 *
 * Mirrors `remarkMessageLinks` trailing-punctuation handling so a URL pasted
 * at end-of-sentence still keeps `.` / `,` / `)` outside the href.
 */
import { createRemarkPrefixPlugin } from "../../../shared/lib/createRemarkPrefixPlugin.ts";

const EDITOR_DEEP_LINK_PATTERN = /(?:cursor|vscode):\/\/[^\s<>"')\]]+/g;
const TRAILING_PUNCTUATION_PATTERN = /[.,;:!?]+$/;

function trimEditorDeepLinkMatch(matchText: string) {
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

export default function remarkEditorDeepLinks() {
  return createRemarkPrefixPlugin(EDITOR_DEEP_LINK_PATTERN, (matchText) => {
    const { value, trailing } = trimEditorDeepLinkMatch(matchText);

    return {
      node: {
        type: "link",
        url: value,
        title: null,
        children: [{ type: "text", value }],
      },
      trailing,
    };
  });
}
