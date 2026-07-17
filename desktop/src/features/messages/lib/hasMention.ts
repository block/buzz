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
 * Handles fenced blocks, four-space/tab-indented lines, and backtick code spans.
 */
function maskMarkdownCode(text: string): string {
  const chars = text.split("");
  const lines: Array<{ start: number; end: number; content: string }> = [];

  let lineStart = 0;
  while (lineStart < text.length) {
    let lineEnd = lineStart;
    while (
      lineEnd < text.length &&
      text[lineEnd] !== "\n" &&
      text[lineEnd] !== "\r"
    ) {
      lineEnd += 1;
    }
    lines.push({
      start: lineStart,
      end: lineEnd,
      content: text.slice(lineStart, lineEnd),
    });
    if (text[lineEnd] === "\r" && text[lineEnd + 1] === "\n") lineEnd += 1;
    lineStart = lineEnd + 1;
  }

  let fence: { marker: string; length: number } | null = null;
  for (const line of lines) {
    if (fence) {
      maskRange(chars, text, line.start, line.end);
      const closing = line.content.match(/^ {0,3}(`+|~+)[ \t]*$/);
      if (
        closing &&
        closing[1][0] === fence.marker &&
        closing[1].length >= fence.length
      ) {
        fence = null;
      }
      continue;
    }

    const opening = line.content.match(/^ {0,3}(`{3,}|~{3,})(.*)$/);
    if (opening && !(opening[1][0] === "`" && opening[2].includes("`"))) {
      fence = { marker: opening[1][0], length: opening[1].length };
      maskRange(chars, text, line.start, line.end);
      continue;
    }

    if (/^(?: {4}|\t)/.test(line.content)) {
      maskRange(chars, text, line.start, line.end);
    }
  }

  const isMasked = (index: number) =>
    chars[index] === " " && text[index] !== " ";
  const isEscaped = (index: number) => {
    let slashCount = 0;
    for (
      let cursor = index - 1;
      cursor >= 0 && text[cursor] === "\\";
      cursor -= 1
    ) {
      slashCount += 1;
    }
    return slashCount % 2 === 1;
  };

  for (let index = 0; index < text.length; ) {
    if (text[index] !== "`" || isMasked(index) || isEscaped(index)) {
      index += 1;
      continue;
    }

    let openerEnd = index + 1;
    while (
      openerEnd < text.length &&
      text[openerEnd] === "`" &&
      !isMasked(openerEnd)
    ) {
      openerEnd += 1;
    }
    const delimiterLength = openerEnd - index;
    let closer = openerEnd;

    while (closer < text.length) {
      if (text[closer] !== "`" || isMasked(closer)) {
        closer += 1;
        continue;
      }
      let closerEnd = closer + 1;
      while (
        closerEnd < text.length &&
        text[closerEnd] === "`" &&
        !isMasked(closerEnd)
      ) {
        closerEnd += 1;
      }
      if (closerEnd - closer === delimiterLength) {
        maskRange(chars, text, index, closerEnd);
        index = closerEnd;
        break;
      }
      closer = closerEnd;
    }

    if (closer >= text.length) index = openerEnd;
  }

  return chars.join("");
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
export function getMentionOffset(text: string, name: string): number | null {
  const escaped = escapeRegExp(name);
  const pattern = new RegExp(
    `(^|\\s|\\(|[*_]{1,3}|\\|\\|)(@${escaped})(?=\\|\\||[\\s,;.!?:)\\]}*_]|$)`,
    "i",
  );
  const match = pattern.exec(maskMarkdownCode(text));
  return match ? match.index + match[1].length : null;
}

export function hasMention(text: string, name: string): boolean {
  return getMentionOffset(text, name) !== null;
}
