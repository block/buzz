/** Return false for oversized text or Unicode controls that can spoof labels. */
export function isSafeDisplayText(value: string, maximum: number): boolean {
  if (new TextEncoder().encode(value).length > maximum) return false;

  for (const character of value) {
    const codePoint = character.codePointAt(0) ?? 0;
    if (
      codePoint <= 0x1f ||
      (codePoint >= 0x7f && codePoint <= 0x9f) ||
      (codePoint >= 0x200b && codePoint <= 0x200f) ||
      (codePoint >= 0x202a && codePoint <= 0x202e) ||
      (codePoint >= 0x2060 && codePoint <= 0x2064) ||
      (codePoint >= 0x2066 && codePoint <= 0x206f) ||
      codePoint === 0xfeff
    ) {
      return false;
    }
  }

  return true;
}
