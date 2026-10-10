import { type Component, Markdown } from "@earendil-works/pi-tui";
import {
  agentName,
  fit,
  markdownTheme,
  safeText,
  singleLine,
  style,
} from "./theme.ts";

/** A message or status event displayed in a conversation transcript. */
export interface TranscriptEntry {
  id: string;
  author: string;
  authorPubkey?: string;
  content: string;
  time: string;
  self?: boolean;
  kind?: "message" | "activity" | "error";
  detail?: string;
}

function safeMarkdown(value: string): string {
  return safeText(value)
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1");
}

/** Width-aware, cached renderer for a bounded conversation history. */
export class Transcript implements Component {
  private entries: TranscriptEntry[];
  private cachedWidth?: number;
  private cachedLines?: string[];

  constructor(entries: TranscriptEntry[] = []) {
    this.entries = entries.slice();
  }

  setEntries(entries: TranscriptEntry[]): void {
    this.entries = entries.slice();
    this.invalidate();
  }

  render(width: number): string[] {
    const available = Math.max(1, width);
    if (this.cachedLines && this.cachedWidth === available)
      return this.cachedLines;
    if (this.entries.length === 0) {
      this.cachedLines = [
        style.muted(fit("Your agents keep working.", available)),
        style.muted(
          fit("Choose a conversation or recipient to begin.", available),
        ),
      ];
      this.cachedWidth = available;
      return this.cachedLines;
    }

    const lines: string[] = [];
    for (const entry of this.entries) {
      const kind = entry.kind ?? "message";
      if (kind === "activity") {
        lines.push(
          fit(
            style.muted(
              `· ${singleLine(entry.content)}${entry.detail ? ` — ${singleLine(entry.detail)}` : ""}`,
            ),
            available,
          ),
        );
        continue;
      }

      if (lines.length > 0) lines.push("");
      const author = singleLine(entry.author) || "Unknown";
      const time = singleLine(entry.time);
      const name = entry.self
        ? style.accent(author)
        : entry.authorPubkey
          ? agentName(entry.authorPubkey, author)
          : style.bold(author);
      const header = `${name}  ${style.muted(time)}`;
      lines.push(
        fit(
          kind === "error" ? style.error(`${author}  ${time}`) : header,
          available,
        ),
      );
      const gutter = kind === "error" ? style.error("! ") : style.muted("│ ");
      const contentWidth = Math.max(1, available - 2);
      const markdown = new Markdown(
        safeMarkdown(entry.content),
        0,
        0,
        markdownTheme,
        { color: style.text },
        { renderLatex: false },
      );
      const rendered = markdown.render(contentWidth);
      for (const line of rendered.length > 0 ? rendered : [""]) {
        // Markdown decodes entities after input sanitization; retain only SGR styles.
        const safe = line
          // biome-ignore lint/suspicious/noControlCharactersInRegex: recognize permitted SGR escapes only
          .split(/(\x1b\[[0-9;]*m)/)
          .map((part) =>
            // biome-ignore lint/suspicious/noControlCharactersInRegex: preserve SGR, sanitize every other control
            /^\x1b\[[0-9;]*m$/.test(part) ? part : safeText(part),
          )
          .join("");
        lines.push(fit(`${gutter}${safe}`, available));
      }
      if (entry.detail)
        lines.push(
          fit(`${gutter}${style.muted(singleLine(entry.detail))}`, available),
        );
    }

    this.cachedWidth = available;
    this.cachedLines = lines;
    return lines;
  }

  invalidate(): void {
    this.cachedWidth = undefined;
    this.cachedLines = undefined;
  }
}
