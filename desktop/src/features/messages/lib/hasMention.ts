import { fromMarkdown } from "mdast-util-from-markdown";

// Structural view of the mdast nodes this file cares about. Declared locally
// because `@types/mdast` is not a dependency here and the full node union is
// far more than a masker needs: a type, source offsets, and children.
type MarkdownNode = {
  type: string;
  position?: {
    start: { offset?: number };
    end: { offset?: number };
  };
  children?: MarkdownNode[];
};

/**
 * Escape special regex characters in a string.
 */
function escapeRegExp(str: string): string {
  return str.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

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
 * The block rules here are not worth reimplementing. An indented code block
 * cannot interrupt a paragraph, but it does not universally need a preceding
 * blank line either — it can open straight after a fenced block, an ATX
 * heading or a thematic break — while `- item` + blank + `    @alice` is a
 * visible second paragraph in the list item, not code. A hand-rolled line
 * classifier gets one of those two classes wrong whichever way it is written,
 * and a mention wrongly kept notifies people (and wakes agents) for text the
 * UI shows as code.
 *
 * So the parser decides. Desktop already depends on `mdast-util-from-markdown`
 * — the same micromark parse `react-markdown` runs to render the message — and
 * we mask the source ranges of its `code` and `inlineCode` nodes. Whatever the
 * renderer puts inside a `<code>`, this masks; the two cannot drift.
 */
function computeMaskedMarkdownCode(text: string): string {
  let tree: MarkdownNode;
  try {
    tree = fromMarkdown(text) as MarkdownNode;
  } catch {
    // A parse failure must not silently un-mask code: leaving the text
    // unmasked is the permissive direction, but the alternative (masking
    // everything) would drop every legitimate mention in the message.
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

// One-entry memo. Mention extraction asks about every member name in the
// community, so a single draft is masked once per name; the parse is the
// expensive part and the text is identical across those calls.
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
  const escaped = escapeRegExp(name);
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
