/**
 * Removes the exact leading prefix synthesized by automatic agent addressing.
 * The captured prefix is provenance: an identical authored mention immediately
 * after it must remain draft content.
 */
export function stripImplicitAgentMentionPrefix(
  content: string,
  implicitPrefix: string,
): string {
  if (!implicitPrefix) return content;
  if (content.startsWith(implicitPrefix)) {
    return content.slice(implicitPrefix.length);
  }
  return content === implicitPrefix.trimEnd() ? "" : content;
}
