/**
 * Remark plugin that detects custom-emoji shortcodes (`:shortcode:`) in text
 * nodes and replaces them with custom HAST `emoji` elements rendered as inline
 * images via react-markdown.
 *
 * Only *known* shortcodes (those present in the provided `customEmoji` map) are
 * matched. Unknown `:foo:` sequences are left as plain text — native unicode
 * emoji are inserted directly by the picker and never pass through here.
 *
 * Mirrors the `remarkMentions` / `remarkChannelLinks` shape but cannot reuse
 * `createRemarkPrefixPlugin`: that factory keys nodes off the matched *text*,
 * whereas here we must carry the resolved image `url` (a side value) onto the
 * node, so the renderer doesn't need the map again.
 */

import { escapeRegExp } from "./mentionPattern";

export type CustomEmoji = {
  shortcode: string;
  url: string;
};

type RemarkCustomEmojiOptions = {
  customEmoji?: CustomEmoji[];
};

// biome-ignore lint/suspicious/noExplicitAny: building mdast-compatible nodes
type Node = { [key: string]: any };

/**
 * Build a regex matching `:shortcode:` for any known shortcode, longest-first
 * so a longer name can't be shadowed by a shorter prefix. Returns null when
 * there are no known shortcodes (nothing to match — skip the walk entirely).
 */
function buildShortcodePattern(shortcodes: string[]): RegExp | null {
  const sorted = [...new Set(shortcodes)]
    .filter((s) => s.trim().length > 0)
    .sort((a, b) => b.length - a.length);
  if (sorted.length === 0) return null;
  const alternatives = sorted.map((s) => escapeRegExp(s)).join("|");
  // Case-insensitive: the set keys are lowercase, but message content may use
  // mixed case (manual typing, other clients). NIP-30 renders by the emoji
  // tag's shortcode, so we match case-insensitively and resolve via lowercase.
  return new RegExp(`:(?:${alternatives}):`, "gi");
}

export default function remarkCustomEmoji(options?: RemarkCustomEmojiOptions) {
  const emoji = options?.customEmoji ?? [];
  const urlByShortcode = new Map(emoji.map((e) => [e.shortcode, e.url]));
  const pattern = buildShortcodePattern(emoji.map((e) => e.shortcode));

  return (
    // biome-ignore lint/suspicious/noExplicitAny: remark tree types are not available
    tree: any,
  ) => {
    if (!pattern) return;
    walkChildren(tree, pattern, urlByShortcode);
  };
}

function walkChildren(
  // biome-ignore lint/suspicious/noExplicitAny: remark tree types are not available
  node: any,
  pattern: RegExp,
  urlByShortcode: Map<string, string>,
) {
  if (
    !node?.children ||
    !Array.isArray(node.children) ||
    shouldSkipNode(node)
  ) {
    return;
  }

  for (let i = node.children.length - 1; i >= 0; i--) {
    const child = node.children[i];
    if (child.type === "text") {
      const parts = splitByPattern(child.value, pattern, urlByShortcode);
      if (
        parts.length > 1 ||
        (parts.length === 1 && parts[0].type !== "text")
      ) {
        node.children.splice(i, 1, ...parts);
      }
    } else {
      walkChildren(child, pattern, urlByShortcode);
    }
  }
}

// biome-ignore lint/suspicious/noExplicitAny: remark tree types are not available
function shouldSkipNode(node: any): boolean {
  return (
    node.type === "link" || node.type === "code" || node.type === "inlineCode"
  );
}

function splitByPattern(
  text: string,
  pattern: RegExp,
  urlByShortcode: Map<string, string>,
): Node[] {
  pattern.lastIndex = 0;
  const parts: Node[] = [];
  let lastIndex = 0;
  let match: RegExpExecArray | null = null;

  while (true) {
    match = pattern.exec(text);
    if (!match) break;

    if (match.index > lastIndex) {
      parts.push({ type: "text", value: text.slice(lastIndex, match.index) });
    }

    const matchText = match[0]; // e.g. ":party_parrot:" or ":Party_Parrot:"
    const shortcode = matchText.slice(1, -1).toLowerCase();
    const url = urlByShortcode.get(shortcode);
    if (url) {
      parts.push(buildEmojiNode(shortcode, url));
    } else {
      // Pattern only matches known shortcodes, so this is defensive; keep raw.
      parts.push({ type: "text", value: matchText });
    }

    lastIndex = match.index + matchText.length;
  }

  if (parts.length === 0) {
    return [{ type: "text", value: text }];
  }
  if (lastIndex < text.length) {
    parts.push({ type: "text", value: text.slice(lastIndex) });
  }
  return parts;
}

function buildEmojiNode(shortcode: string, url: string): Node {
  return {
    type: "emoji",
    value: `:${shortcode}:`,
    data: {
      hName: "emoji",
      hProperties: {
        src: url,
        alt: `:${shortcode}:`,
        "data-shortcode": shortcode,
      },
    },
  };
}
