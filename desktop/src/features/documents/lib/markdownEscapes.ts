/**
 * Undoing prosemirror-markdown's text escaping.
 *
 * `prosemirror-markdown`'s serializer backslash-escapes markdown special
 * characters in text nodes so they can't be reinterpreted as formatting. For
 * chat that is correct. For a vault note it is not: `[[wikilink]]` comes back
 * as `\[\[wikilink\]\]`, and writing that to disk corrupts the link in
 * Obsidian.
 *
 * Onyx patched this in three separate places with three slightly different
 * regexes; Buzz's chat composer has a fourth. This is the one owner.
 */

/**
 * The union of what Onyx stripped (`[ ] _ !`) and what Buzz's composer strips
 * (`` ` * \ ~ [ ] _ ``).
 */
const ESCAPED_MARKDOWN_CHARACTER = /\\([`*\\~[\]_!])/g;

/**
 * Strips one level of backslash escaping from markdown special characters.
 *
 * **This is lossy in one direction and that is a known trade.** A note
 * containing a deliberate literal `\*not bold\*` becomes `*not bold*` and will
 * render as emphasis the next time it is opened — a silent, compounding
 * rewrite. The round-trip guard is the real backstop: a file with deliberate
 * escapes fails the parse/serialize comparison, opens in source mode, and never
 * reaches this function.
 */
export function stripMarkdownEscapes(markdown: string): string {
  return markdown.replace(ESCAPED_MARKDOWN_CHARACTER, "$1");
}

/**
 * tiptap-markdown emits CommonMark hard line breaks as a trailing backslash.
 * Vault notes use plain newlines, so collapse them.
 */
export function normalizeHardBreaks(markdown: string): string {
  return markdown.replace(/\\\n/g, "\n");
}

/** The full editor-output → on-disk text pipeline. */
export function toDiskMarkdown(editorMarkdown: string): string {
  return stripMarkdownEscapes(normalizeHardBreaks(editorMarkdown));
}
