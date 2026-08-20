/**
 * Extract the first non-empty line from a system prompt for use as a
 * short preview string. Returns the trimmed input when every line is empty.
 */
export function promptPreview(prompt: string): string {
  const trimmed = prompt.trim();
  if (!trimmed) {
    return "";
  }
  const [firstLine] = trimmed
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
  return firstLine ?? trimmed;
}
