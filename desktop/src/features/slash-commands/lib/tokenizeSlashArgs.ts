/**
 * Splits an argument string on whitespace. Double quotes group words into one
 * token, and a backslash escapes a quote or backslash inside quotes. An
 * unterminated quote runs to the end of the string.
 */
export function tokenizeSlashArgs(args: string): string[] {
  const tokens: string[] = [];
  let current = "";
  let hasToken = false;
  let inQuotes = false;
  for (let index = 0; index < args.length; index++) {
    const char = args[index];
    if (inQuotes) {
      const next = args[index + 1];
      if (char === "\\" && (next === '"' || next === "\\")) {
        current += next;
        index++;
      } else if (char === '"') {
        inQuotes = false;
      } else {
        current += char;
      }
      continue;
    }
    if (char === '"') {
      inQuotes = true;
      hasToken = true;
    } else if (/\s/.test(char)) {
      if (hasToken) tokens.push(current);
      current = "";
      hasToken = false;
    } else {
      current += char;
      hasToken = true;
    }
  }
  if (hasToken) tokens.push(current);
  return tokens;
}
