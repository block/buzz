/** `¯\_(ツ)_/¯` escaped so the markdown renderer shows it verbatim. */
export const SHRUG_MARKDOWN = "¯\\\\\\_(ツ)\\_/¯";

export function buildShrugMessage(text: string): string {
  const trimmed = text.trim();
  return trimmed ? `${trimmed} ${SHRUG_MARKDOWN}` : SHRUG_MARKDOWN;
}
