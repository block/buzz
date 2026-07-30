/**
 * IRC-style `s/old/new/` self-correction.
 *
 * When a user sends a message whose entire body is a well-formed substitution
 * command, the composer treats it as shorthand for editing their most recent
 * message instead of sending literal text — the same gesture as right-click
 * "Edit message", just faster to type. This module is the pure, UI-free core:
 * it parses the command and applies it to a string. All the wiring (finding the
 * target message, republishing the edit event) lives in the composer.
 *
 * Grammar (sed-flavoured, but **literal text — never regex**):
 *
 *   s<D>pattern<D>replacement<D><flags>
 *
 * - `<D>` is the delimiter, and it is always `/` — the canonical IRC/sed
 *   shorthand (`s/old/new/`). sed itself allows any character as the delimiter,
 *   but the only thing that buys you is avoiding escapes when the delimiter
 *   appears in your pattern — which `\/` escaping already covers. Fixing it to
 *   `/` keeps ordinary prose (`s3 bucket`, `s: notes`, `s|foo`) from ever
 *   looking like a command, shrinking the accidental-trigger surface.
 * - The trailing delimiter is optional when there are no flags (`s/a/b` works).
 * - `\<D>` inside pattern/replacement is a literal delimiter; `\\` is a literal
 *   backslash. Every other backslash is kept verbatim.
 * - Flags: `g` (replace every occurrence, not just the first) and `i`
 *   (case-insensitive match). Each may appear at most once, in any order.
 *
 * Anything that does not parse cleanly returns `null`, so the caller falls back
 * to sending the text literally — a mistyped command is never silently eaten.
 */

export type SelfCorrectionCommand = {
  /** Literal text to search for. Guaranteed non-empty. */
  pattern: string;
  /** Literal text to substitute in. May be empty (a deletion). */
  replacement: string;
  /** Replace every occurrence rather than only the first. */
  global: boolean;
  /** Match case-insensitively. */
  ignoreCase: boolean;
};

/** The one supported delimiter — the canonical IRC/sed `s/old/new/` slash. */
const DELIMITER = "/";

/**
 * Scan a delimited section starting at `start`, honouring `\<delim>` and `\\`
 * escapes. Returns the unescaped section text and the index of the character
 * immediately after the closing delimiter, or `null` if no closing delimiter
 * is found before the end of the string.
 */
function scanSection(
  input: string,
  start: number,
  delimiter: string,
): { value: string; next: number } | null {
  let value = "";
  let index = start;
  while (index < input.length) {
    const char = input[index];
    if (char === "\\" && index + 1 < input.length) {
      const escaped = input[index + 1];
      if (escaped === delimiter || escaped === "\\") {
        value += escaped;
        index += 2;
        continue;
      }
    }
    if (char === delimiter) {
      return { value, next: index + 1 };
    }
    value += char;
    index += 1;
  }
  return null;
}

/**
 * Parse a trimmed message body as a self-correction command. Returns `null`
 * when the text is not a well-formed command (including the empty pattern
 * case), signalling the caller to treat the text as an ordinary message.
 */
export function parseSelfCorrection(
  input: string,
): SelfCorrectionCommand | null {
  // `s/` + at least a closing delimiter is the minimum shape.
  if (input.length < 3 || input[0] !== "s" || input[1] !== DELIMITER) {
    return null;
  }
  const delimiter = DELIMITER;

  const patternSection = scanSection(input, 2, delimiter);
  if (!patternSection || patternSection.value.length === 0) {
    return null;
  }

  const replacementSection = scanSection(input, patternSection.next, delimiter);

  let replacement: string;
  let flagsStart: number;
  if (replacementSection) {
    replacement = replacementSection.value;
    flagsStart = replacementSection.next;
  } else {
    // No closing delimiter after the replacement: allowed only when it runs to
    // the end of the string (the trailing-delimiter-omitted form, no flags).
    // Re-scan without requiring a terminator, unescaping as we go.
    replacement = unescapeToEnd(input, patternSection.next, delimiter);
    flagsStart = input.length;
  }

  let global = false;
  let ignoreCase = false;
  for (let index = flagsStart; index < input.length; index += 1) {
    const flag = input[index];
    if (flag === "g" && !global) {
      global = true;
    } else if (flag === "i" && !ignoreCase) {
      ignoreCase = true;
    } else {
      return null;
    }
  }

  return { pattern: patternSection.value, replacement, global, ignoreCase };
}

/** Unescape `\<delim>` / `\\` from `start` to end of string. */
function unescapeToEnd(
  input: string,
  start: number,
  delimiter: string,
): string {
  let value = "";
  let index = start;
  while (index < input.length) {
    const char = input[index];
    if (char === "\\" && index + 1 < input.length) {
      const escaped = input[index + 1];
      if (escaped === delimiter || escaped === "\\") {
        value += escaped;
        index += 2;
        continue;
      }
    }
    value += char;
    index += 1;
  }
  return value;
}

/**
 * Apply a parsed command to `text`. Returns the corrected string, or `null`
 * when the pattern does not occur in `text` (nothing to correct — the caller
 * should fall back to sending the text literally).
 */
export function applySelfCorrection(
  text: string,
  command: SelfCorrectionCommand,
): string | null {
  const { pattern, replacement, global, ignoreCase } = command;
  // Search over a case-folded copy when ignoring case, but splice from the
  // original so the surrounding text keeps its casing. Index alignment holds
  // for the common (ASCII / BMP) case; exotic case-folding that changes length
  // is not supported and is acceptably rare for a typo-fix affordance.
  const haystack = ignoreCase ? text.toLowerCase() : text;
  const needle = ignoreCase ? pattern.toLowerCase() : pattern;

  let matchAt = haystack.indexOf(needle);
  if (matchAt === -1) {
    return null;
  }

  let result = "";
  let copiedUpTo = 0;
  while (matchAt !== -1) {
    result += text.slice(copiedUpTo, matchAt) + replacement;
    copiedUpTo = matchAt + pattern.length;
    if (!global) {
      break;
    }
    // Resume past this match so replacements never overlap or re-match inside
    // the substituted text. `needle.length === pattern.length` (case-folding
    // preserves length for the supported character range), so `copiedUpTo` is a
    // valid index into `haystack`.
    matchAt = haystack.indexOf(needle, copiedUpTo);
  }
  result += text.slice(copiedUpTo);
  return result;
}
