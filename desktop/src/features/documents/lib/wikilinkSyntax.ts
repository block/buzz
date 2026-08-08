/**
 * Obsidian wikilink syntax — the single owner.
 *
 * Onyx carries two different regexes for this: one in the editor plugin that
 * splits `[[Note#Heading]]` into a target and a heading, and one in the note
 * index that captures `Note#Heading` whole. They disagree, so the editor
 * renders a link the graph never records and the backlink silently goes
 * missing. Everything here parses through one pattern.
 */

export type Wikilink = {
  /** Note name, or `""` for a same-note anchor like `[[#Heading]]`. */
  target: string;
  /** Heading anchor without the `#`, or `null`. */
  heading: string | null;
  /** Block id without the `^`, or `null`. */
  blockId: string | null;
  /** Display text after `|`, or `null`. */
  alias: string | null;
  /** The matched source text, including brackets and any escaping. */
  raw: string;
  /** Offset of the match within the searched string. */
  index: number;
};

/**
 * Matches every wikilink form Obsidian accepts:
 *
 *   [[Note]]                  [[Note|alias]]
 *   [[Note#Heading]]          [[Note#Heading|alias]]
 *   [[Note^blockid]]          [[Note#^blockid]]
 *   [[#Heading]]              [[^blockid]]        (same-note anchors)
 *
 * Two details that are easy to get wrong:
 *
 *  - `\[\[…\]\]` is tolerated. prosemirror-markdown escapes brackets in text
 *    nodes, so a link read back out of the serializer arrives escaped.
 *  - Embeds (`![[…]]`) are excluded, being a different construct. That needs
 *    *two* lookbehinds: `(?<!!)` catches `![[x]]`, and `(?<!!\\)` catches the
 *    escaped `!\[\[x\]\]`, where the match would otherwise start one character
 *    in — at the `[` after the backslash — and see only `\` behind it.
 *
 * `#^` is tried before bare `#` so `[[Note#^id]]` reads as a block reference
 * rather than a heading named `^id`. Backslashes are excluded from the target
 * so a greedy match cannot absorb the closing `\]`'s escape, and newlines are
 * excluded throughout so an unclosed `[[` cannot swallow the rest of the file.
 */
const WIKILINK_PATTERN =
  /(?<!!)(?<!!\\)\\?\[\\?\[([^\]#|^\n\\]*)(?:#\^([^\]|\n]+))?(?:#([^\]|^\n\\]+))?(?:\^([^\]|\n\\]+))?(?:\|([^\]\n\\]+))?\\?\]\\?\]/g;

function normalize(value: string | undefined): string | null {
  const trimmed = value?.trim();
  return trimmed ? trimmed : null;
}

/** Every wikilink in `text`, in source order. */
export function parseWikilinks(text: string): Wikilink[] {
  const links: Wikilink[] = [];
  // A fresh regex per call keeps `lastIndex` from leaking between callers —
  // the classic bug with a shared global-flagged pattern.
  const pattern = new RegExp(WIKILINK_PATTERN.source, "g");

  let match: RegExpExecArray | null = pattern.exec(text);
  while (match !== null) {
    const [raw, target, blockViaHash, heading, blockViaCaret, alias] = match;
    const parsed: Wikilink = {
      alias: normalize(alias),
      blockId: normalize(blockViaHash) ?? normalize(blockViaCaret),
      heading: normalize(heading),
      index: match.index,
      raw,
      target: target?.trim() ?? "",
    };

    // `[[]]` carries no destination at all; skip rather than emit a link to
    // nothing.
    if (parsed.target || parsed.heading || parsed.blockId) {
      links.push(parsed);
    }
    match = pattern.exec(text);
  }

  return links;
}

/**
 * Distinct note names linked from `text`.
 *
 * Same-note anchors (`[[#Heading]]`) contribute no target and are excluded, so
 * a note never appears to link to itself merely for having internal anchors.
 */
export function extractLinkTargets(text: string): string[] {
  const seen = new Set<string>();
  for (const link of parseWikilinks(text)) {
    if (link.target) seen.add(link.target);
  }
  return [...seen];
}

/** How a wikilink should be displayed. */
export function wikilinkDisplayText(link: Wikilink): string {
  if (link.alias) return link.alias;
  if (link.target && link.heading) return `${link.target} › ${link.heading}`;
  if (link.target) return link.target;
  if (link.heading) return link.heading;
  return link.blockId ?? "";
}
