/**
 * Obsidian inline and block syntax that the editor decorates but does not own.
 *
 * Everything here is recognised by pattern and styled with a ProseMirror
 * decoration, never converted into a schema node. That is a deliberate
 * constraint, not a shortcut: a real node would have to serialize itself back,
 * and any imperfection there means the round-trip guard starts routing
 * perfectly good notes into source mode. Decorations leave the text untouched,
 * so `==highlight==` on disk is still `==highlight==` after a save.
 */

export type InlineMatch = {
  /** Offset within the searched text. */
  index: number;
  /** Full matched text, including delimiters. */
  raw: string;
  /** The content between the delimiters. */
  content: string;
};

export type CalloutType = {
  /** Lowercased type from `> [!type]`. */
  type: string;
  /** The canonical type this aliases to. */
  canonical: string;
  /** Title text after the marker, if the author supplied one. */
  title: string | null;
};

/**
 * `==highlight==`. Requires non-space at both inner edges so `== ==` and a
 * stray `====` separator are not treated as highlights.
 */
const HIGHLIGHT_PATTERN = /==(?!\s)((?:[^=]|=(?!=))+?)(?<!\s)==/g;

/** `%%comment%%` — Obsidian's "not rendered" marker. */
const COMMENT_PATTERN = /%%(?!\s)((?:[^%]|%(?!%))+?)(?<!\s)%%/g;

/**
 * `#tag`, including `#nested/tag`.
 *
 * Must not match a markdown heading (`# Title` has a space), a bare `#`, or a
 * CSS colour (`#fff`) — hence requiring at least one non-digit. The preceding
 * character must be start-of-string or whitespace so `foo#bar` and a URL
 * fragment are left alone.
 */
const TAG_PATTERN =
  /(?<=^|\s)#(?![0-9]+(?:\s|$))([A-Za-z0-9_\-/]*[A-Za-z_][A-Za-z0-9_\-/]*)/g;

function collect(pattern: RegExp, text: string): InlineMatch[] {
  // A fresh regex per call: a shared global-flagged pattern carries `lastIndex`
  // between callers and silently skips matches.
  const local = new RegExp(pattern.source, pattern.flags);
  const matches: InlineMatch[] = [];

  let match: RegExpExecArray | null = local.exec(text);
  while (match !== null) {
    matches.push({ content: match[1], index: match.index, raw: match[0] });
    match = local.exec(text);
  }
  return matches;
}

export function findHighlights(text: string): InlineMatch[] {
  return collect(HIGHLIGHT_PATTERN, text);
}

export function findComments(text: string): InlineMatch[] {
  return collect(COMMENT_PATTERN, text);
}

export function findTags(text: string): InlineMatch[] {
  return collect(TAG_PATTERN, text);
}

/**
 * `^block-id` — a block reference anchor, valid only at the end of a line.
 *
 * Must not match a caret inside a wikilink (`[[Note^id]]` is the *link*, not an
 * anchor), so callers filter those out; see `findBlockIds`.
 */
const BLOCK_ID_PATTERN = /(?<=\s)\^([A-Za-z0-9-]+)\s*$/;

/**
 * The trailing block anchor on a line, or `null`.
 *
 * Onyx's plugin scans for these and then separately excludes ones inside
 * wikilinks; anchoring to end-of-line makes that exclusion automatic, because a
 * wikilink's caret is always followed by `]]`.
 */
export function findBlockId(line: string): InlineMatch | null {
  const match = BLOCK_ID_PATTERN.exec(line);
  if (!match) return null;
  return { content: match[1], index: match.index, raw: match[0].trimEnd() };
}

/**
 * Obsidian's callout aliases, mapped to the canonical type whose styling they
 * share. Ported from Onyx's table.
 */
const CALLOUT_ALIASES: Record<string, string> = {
  abstract: "summary",
  attention: "warning",
  bug: "bug",
  caution: "warning",
  check: "success",
  cite: "quote",
  danger: "danger",
  done: "success",
  error: "danger",
  example: "example",
  fail: "failure",
  failure: "failure",
  faq: "question",
  help: "question",
  hint: "tip",
  important: "tip",
  info: "info",
  missing: "failure",
  note: "note",
  question: "question",
  quote: "quote",
  success: "success",
  summary: "summary",
  tip: "tip",
  todo: "todo",
  tldr: "summary",
  warning: "warning",
};

/** `> [!info] Optional title` — only valid on the first line of a blockquote. */
const CALLOUT_PATTERN = /^>\s*\[!([A-Za-z]+)\][+-]?\s*(.*)$/;

/** Parses a callout marker, or returns `null` when the line is not one. */
export function parseCallout(line: string): CalloutType | null {
  const match = CALLOUT_PATTERN.exec(line);
  if (!match) return null;

  const type = match[1].toLowerCase();
  const canonical = CALLOUT_ALIASES[type];
  // An unknown type is still a callout — Obsidian renders it with default
  // styling rather than as a plain quote.
  return {
    canonical: canonical ?? "note",
    title: match[2].trim() || null,
    type,
  };
}

export type OutlineHeading = {
  /** 1-6. */
  level: number;
  text: string;
  /** ProseMirror document position of the heading node. */
  position: number;
};

/**
 * Picks the active outline entry for a scroll offset.
 *
 * The last heading at or above the viewport top wins; before the first
 * heading, nothing is active.
 */
export function activeHeadingIndex(
  offsets: readonly number[],
  scrollTop: number,
): number {
  let active = -1;
  for (const [index, offset] of offsets.entries()) {
    // A small tolerance stops the active item flickering when a heading sits
    // exactly on the viewport edge.
    if (offset - 8 <= scrollTop) {
      active = index;
    } else {
      break;
    }
  }
  return active;
}
