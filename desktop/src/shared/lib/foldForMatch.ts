/**
 * Fold text for accent- and case-insensitive name matching, so "elodie"
 * finds "Élodie" and "reunion" finds "Réunion".
 *
 * Each code point is lower-cased, decomposed (NFD) with its combining marks
 * removed, then letters that NFD leaves whole are mapped to plain ASCII:
 * `ø`→`o`, `đ`→`d`, `ı`→`i`, `œ`→`oe`, `æ`→`ae`, `ß`→`ss`.
 *
 * Use it on both sides of a comparison only. Never display, insert, or key
 * identities on folded text.
 */
export function foldForMatch(s: string): string {
  let folded = "";
  for (const character of s) {
    folded += character
      .toLowerCase()
      .normalize("NFD")
      .replace(/\p{M}/gu, "")
      .replace(/ø/g, "o")
      .replace(/đ/g, "d")
      .replace(/ı/g, "i")
      .replace(/œ/g, "oe")
      .replace(/æ/g, "ae")
      .replace(/ß/g, "ss");
  }
  return folded;
}
