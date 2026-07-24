/**
 * Normalize one finalized STT segment for insertion at the editor selection.
 *
 * Parakeet emits complete utterances without leading/trailing whitespace. Add
 * a boundary when dictating after existing text and retain one trailing space
 * so the next finalized segment joins naturally.
 */
export function buildDictationInsertion(
  previousCharacter: string,
  transcript: string,
): string {
  const normalized = transcript.trim();
  if (!normalized) return "";
  const prefix =
    previousCharacter.length > 0 && !/\s/.test(previousCharacter) ? " " : "";
  return `${prefix}${normalized} `;
}
