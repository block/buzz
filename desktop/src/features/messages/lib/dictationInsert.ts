/**
 * Spacing rules for inserting a dictated segment at the caret.
 *
 * The STT pipeline emits one finalized segment per speech burst, with no
 * leading or trailing whitespace and no inter-segment spacing. Dictating
 * "hello there" then "how are you" must not produce "hello therehow are you",
 * but it also must not add a space at the start of an empty composer or
 * double one the user already typed.
 */
export function buildDictationInsert(
  precedingText: string,
  segment: string,
): string {
  const trimmed = segment.trim();
  if (!trimmed) return "";
  // Start of the composer, or the user already left whitespace (including a
  // newline) — insert as-is.
  if (precedingText.length === 0 || /\s$/.test(precedingText)) return trimmed;
  return ` ${trimmed}`;
}
