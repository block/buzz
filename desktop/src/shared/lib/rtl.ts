/**
 * Message-level RTL support for Arabic-script text (Arabic, Persian/Farsi,
 * Urdu). The direction of a paragraph follows the first strong directional
 * character typed in it. When the first paragraph is neutral (no strong
 * character), the first paragraph with a strong RTL character decides for
 * the whole message. Pure LTR content is unaffected.
 */

const RTL_CHAR_RANGE =
  "[\u0600-\u06FF\u0750-\u077F\u08A0-\u08FF\uFB50-\uFDFF\uFE70-\uFEFF]";
const RTL_LEADING_RE = new RegExp(
  `^[^A-Za-z\u00C0-\u024F]*${RTL_CHAR_RANGE}`,
);

/** Split content into paragraphs (blank-line separated blocks). */
function splitParagraphs(content: string): string[] {
  return content
    .split(/\n[ \t]*\n/)
    .filter((p) => p.trim().length > 0);
}

/** True when [paragraph] itself starts with a strong RTL character. */
function isRtlParagraph(paragraph: string): boolean {
  // Markdown emphasis/link prefixes must not hide the first real letter.
  return RTL_LEADING_RE.test(paragraph);
}

/**
 * True when [content] should be laid out right-to-left: the first
 * paragraph decides; if it is neutral, the first paragraph with a strong
 * RTL character decides.
 */
export function isRtlContent(content: string | null | undefined): boolean {
  if (!content) return false;
  const paragraphs = splitParagraphs(content);
  if (paragraphs.length === 0) return false;
  if (isRtlParagraph(paragraphs[0])) return true;
  return paragraphs.slice(1).some(isRtlParagraph);
}
