/** How many characters `text` has, counting code points rather than units. */
export function countCharacters(text: string): number {
  return [...text].length;
}

/**
 * Truncate `text` to at most `maxCharacters`, cutting between characters
 * rather than between the UTF-16 code units a character is stored in.
 *
 * `String.prototype.slice` counts code units. Most emoji, and plenty of CJK,
 * live outside the Basic Multilingual Plane and are stored as a surrogate
 * pair, so a cut that lands inside one leaves a lone surrogate: not a
 * character, and rendered as `�` at the end of the preview.
 *
 * Guard with `countCharacters`, not `.length`. Deciding in code units and
 * cutting in code points disagree on emoji-heavy text: a 150-emoji string is
 * 300 units long, so a `length > 200` guard fires while this returns the
 * string untouched — and the caller appends an ellipsis to complete text.
 *
 * Characters here means code points, not grapheme clusters. A cut can still
 * land between the parts of a ZWJ sequence (a family emoji becoming one
 * person), which is a different picture but a valid string — unlike the lone
 * surrogate, which is not text at all.
 */
export function truncateByCharacters(
  text: string,
  maxCharacters: number,
): string {
  if (maxCharacters <= 0) return "";
  const characters = [...text];
  if (characters.length <= maxCharacters) return text;
  return characters.slice(0, maxCharacters).join("");
}
