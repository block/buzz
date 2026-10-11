/**
 * Remove empty multiline spoiler shells without a backtracking regular
 * expression. The single forward scan is linear in the number of lines.
 */
export function removeEmptySpoilerBlocks(value: string): string {
  const lines = value.split("\n");
  const retained: string[] = [];
  for (let index = 0; index < lines.length; ) {
    if (lines[index]?.trim() === "||") {
      let closingIndex = index + 1;
      while (
        closingIndex < lines.length &&
        lines[closingIndex]?.trim() === ""
      ) {
        closingIndex += 1;
      }
      if (lines[closingIndex]?.trim() === "||") {
        index = closingIndex + 1;
        continue;
      }
    }
    retained.push(lines[index] ?? "");
    index += 1;
  }
  return retained.join("\n");
}
