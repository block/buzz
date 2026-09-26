import type { EditorTheme, MarkdownTheme } from "@earendil-works/pi-tui";
import {
  stripTerminalSequences,
  truncateToWidth,
} from "@earendil-works/pi-tui";

const color =
  (code: string) =>
  (value: string): string =>
    process.env.NO_COLOR === undefined ? `\x1b[${code}m${value}\x1b[0m` : value;

/** Remove terminal commands and directional text controls from untrusted text. */
export function safeText(input: string): string {
  let result = "";
  for (const character of stripTerminalSequences(
    input.replace(/\r\n?/g, "\n"),
  )) {
    const code = character.codePointAt(0) ?? 0;
    const allowedWhitespace = code === 0x09 || code === 0x0a;
    const terminalControl =
      !allowedWhitespace && (code < 0x20 || (code >= 0x7f && code <= 0x9f));
    const bidiControl =
      code === 0x061c ||
      code === 0x200e ||
      code === 0x200f ||
      (code >= 0x202a && code <= 0x202e) ||
      (code >= 0x2066 && code <= 0x2069);
    if (!terminalControl && !bidiControl) result += character;
  }
  return result;
}

/** Convert untrusted text to one display line. */
export function singleLine(input: string): string {
  return safeText(input)
    .replace(/[\n\t]+/g, " ")
    .replace(/\s{2,}/g, " ")
    .trim();
}

/** Shared, palette-friendly terminal styles. */
export const style = {
  accent: color("33"),
  muted: color("2;37"),
  secondary: color("38;5;244"),
  border: color("38;5;240"),
  selected: color("1;30;43"),
  text: color("39"),
  success: color("32"),
  error: color("31"),
  bold: color("1"),
} as const;

/** Keep identity colors stable across views, profile renames, and reconnects. */
export function agentName(pubkey: string, name: string): string {
  const colors = [81, 114, 213, 179, 147, 209, 80];
  const hash = [...pubkey].reduce(
    (value, character) => (value * 31 + character.charCodeAt(0)) >>> 0,
    0,
  );
  return color(`1;38;5;${colors[hash % colors.length]}`)(singleLine(name));
}

/** Remove foreground styles but preserve renderer cursor markers behind overlays. */
export function dimBackground(line: string): string {
  // biome-ignore lint/suspicious/noControlCharactersInRegex: replace SGR only, not cursor markers
  return style.muted(line.replace(/\x1b\[[0-9;]*m/g, ""));
}

export const editorTheme: EditorTheme = {
  borderColor: style.accent,
  selectList: {
    selectedPrefix: style.accent,
    selectedText: style.accent,
    description: style.muted,
    scrollInfo: style.muted,
    noMatch: style.muted,
  },
};

export const markdownTheme: MarkdownTheme = {
  heading: style.bold,
  link: style.accent,
  linkUrl: style.muted,
  code: style.accent,
  codeBlock: style.text,
  codeBlockBorder: style.muted,
  quote: style.text,
  quoteBorder: style.muted,
  hr: style.muted,
  listBullet: style.accent,
  bold: style.bold,
  italic: (value) => color("3")(value),
  strikethrough: (value) => color("9")(value),
  underline: (value) => color("4")(value),
  highlightCode: (code, language) =>
    code
      .split("\n")
      .map((line) =>
        language === "diff" && line.startsWith("+")
          ? style.success(line)
          : language === "diff" && line.startsWith("-")
            ? style.error(line)
            : style.text(line),
      ),
  codeBlockIndent: "  ",
};

/** Truncate trusted styled text to terminal columns without splitting graphemes. */
export function fit(text: string, width: number): string {
  return truncateToWidth(text, Math.max(0, width), "…");
}
