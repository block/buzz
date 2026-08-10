/**
 * YAML frontmatter splitting.
 *
 * This is not a parser and does not want to be — v1 has no Properties UI. The
 * only job is to keep the frontmatter block *out* of the editor and put it back
 * byte-for-byte on save.
 *
 * That matters more than it sounds. Round-tripping `---\ntitle: Note\n---`
 * through tiptap-markdown yields `---\n\n## title: Note`: the opening fence
 * becomes a thematic break and every YAML line becomes a heading. Splitting it
 * off before the editor ever sees it removes the single largest source of
 * silent corruption in a real Obsidian vault.
 */

export type SplitDocument = {
  /**
   * The frontmatter block including its delimiters and trailing newline, or
   * `null` when the file has none. Preserved verbatim — never reformatted.
   */
  frontmatter: string | null;
  /** Everything after the frontmatter block. */
  body: string;
};

/**
 * A frontmatter block must start on the very first line, and the delimiter is
 * exactly three dashes on a line of their own. Obsidian and Jekyll both allow a
 * trailing `\r`, so tolerate CRLF.
 */
const OPENING_FENCE = /^---[ \t]*\r?\n/;

export function splitFrontmatter(raw: string): SplitDocument {
  const opening = OPENING_FENCE.exec(raw);
  if (!opening) {
    return { body: raw, frontmatter: null };
  }

  // Find the closing fence, starting the search after the opening one.
  const searchFrom = opening[0].length;
  const closing = /^---[ \t]*(\r?\n|$)/m.exec(raw.slice(searchFrom));
  if (!closing) {
    // An unterminated block is not frontmatter — treat the whole file as body
    // rather than swallowing it.
    return { body: raw, frontmatter: null };
  }

  let end = searchFrom + closing.index + closing[0].length;

  // Absorb the blank lines that conventionally separate the block from the
  // body. The editor drops a leading blank line, so leaving it on the body
  // side would make every note with frontmatter fail the round-trip guard and
  // open in source mode — for a purely cosmetic difference.
  const separator = /^(?:[ \t]*\r?\n)+/.exec(raw.slice(end));
  if (separator) {
    end += separator[0].length;
  }

  return {
    body: raw.slice(end),
    frontmatter: raw.slice(0, end),
  };
}

/**
 * Re-attaches a frontmatter block to an edited body.
 *
 * `splitFrontmatter` keeps the block's own trailing newline, so this is a plain
 * concatenation — which is exactly the point: whatever the user's YAML looked
 * like, byte-for-byte, is what goes back to disk.
 */
export function joinFrontmatter(
  frontmatter: string | null,
  body: string,
): string {
  if (!frontmatter) return body;
  return `${frontmatter}${body}`;
}
