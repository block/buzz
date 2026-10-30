import { fromMarkdown } from "mdast-util-from-markdown";

/**
 * Escape special regex characters in a string.
 */
function escapeRegExp(str: string): string {
  return str.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** Literal label grammar shared by routing and rendered mention tokens.
 * A full-key qualification (and its numeric reservation suffix) belongs to the
 * label, never to a shorter prefix, even when that longer label is unbound.
 */
export function mentionLabelPattern(label: string): string {
  const qualification = `(?! \\([0-9a-f]{64}\\))`;
  const suffix = / \([0-9a-f]{64}\)$/i.test(label)
    ? `(?! (?:[2-9]|[1-9][0-9]+)(?=[\\s,;.!?:)\\]}*_]|$))`
    : "";
  return `${escapeRegExp(label)}${qualification}${suffix}`;
}

type MarkdownNode = {
  type: string;
  position?: {
    start: { offset?: number };
    end: { offset?: number };
  };
  children?: MarkdownNode[];
};

function maskRange(
  chars: string[],
  text: string,
  start: number,
  end: number,
): void {
  for (let index = start; index < end; index += 1) {
    if (text[index] !== "\n" && text[index] !== "\r") chars[index] = " ";
  }
}

/**
 * Replace Markdown code with spaces while retaining offsets and line endings.
 *
 * A four-space line is not always code: nested list items and a list item's
 * second paragraph indent the same way, and indented code can open with no
 * blank line after a heading, fence, or thematic break. The micromark parse
 * `react-markdown` uses is already a Desktop dependency, so we mask the source
 * ranges of its `code` and `inlineCode` nodes instead of classifying lines.
 */
function computeMaskedMarkdownCode(text: string): string {
  let tree: MarkdownNode;
  try {
    tree = fromMarkdown(text) as MarkdownNode;
  } catch {
    return text;
  }

  const chars = text.split("");
  const visit = (node: MarkdownNode): void => {
    const start = node.position?.start.offset;
    const end = node.position?.end.offset;
    if (
      (node.type === "code" || node.type === "inlineCode") &&
      start !== undefined &&
      end !== undefined
    ) {
      maskRange(chars, text, start, end);
      return;
    }
    for (const child of node.children ?? []) {
      visit(child);
    }
  };
  visit(tree);
  return chars.join("");
}

let maskedCache: { text: string; masked: string } | null = null;

function maskMarkdownCode(text: string): string {
  if (maskedCache?.text === text) return maskedCache.masked;
  const masked = computeMaskedMarkdownCode(text);
  maskedCache = { text, masked };
  return masked;
}

/**
 * Check whether `text` contains an @mention of `name`.
 *
 * Matches `@Name` preceded by start-of-string, whitespace, an opening
 * parenthesis (for team expansions), markdown
 * bold/italic markers (`*`, `**`, `***`, `_`, `__`, `___`), or spoiler
 * delimiters (`||`). This handles the case where a mention is pasted from the
 * chat area and TipTap's Bold extension wraps it in bold marks (font-weight >=
 * 500 -> bold), plus messages whose visible mention text is spoilered.
 *
 * Exported separately so it can be unit-tested without importing React.
 */
export function getMentionOffsets(text: string, name: string): number[] {
  const escaped = mentionLabelPattern(name);
  const pattern = new RegExp(
    `(^|\\s|\\(|[*_]{1,3}|\\|\\|)(@${escaped})(?=\\|\\||[\\s,;.!?:)\\]}*_]|$)`,
    "gi",
  );
  const maskedText = maskMarkdownCode(text);
  const offsets: number[] = [];
  let match = pattern.exec(maskedText);
  while (match !== null) {
    offsets.push(match.index + match[1].length);
    match = pattern.exec(maskedText);
  }
  return offsets;
}

export function getMentionOffset(text: string, name: string): number | null {
  return getMentionOffsets(text, name)[0] ?? null;
}

export function hasMention(text: string, name: string): boolean {
  return getMentionOffset(text, name) !== null;
}
