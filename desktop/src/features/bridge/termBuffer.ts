// Minimal terminal text model for the web terminal tab (no xterm.js dependency installed yet; see report).
// Handles newline, carriage return, backspace, erase-in-line/display and drops other escape sequences.
export class TermBuffer {
  lines: string[] = [""];
  private col = 0;
  private esc = "";
  private inEsc: null | "csi" | "osc" | "esc" = null;
  maxLines: number;
  constructor(maxLines = 2000) {
    this.maxLines = maxLines;
  }

  write(text: string) {
    for (const ch of text) this.put(ch);
    if (this.lines.length > this.maxLines) this.lines.splice(0, this.lines.length - this.maxLines);
  }

  private setCol(line: string, col: number, ch: string) {
    const padded = line.padEnd(col, " ");
    return padded.slice(0, col) + ch + padded.slice(col + 1);
  }

  private put(ch: string) {
    if (this.inEsc === "esc") {
      if (ch === "[") this.inEsc = "csi";
      else if (ch === "]") this.inEsc = "osc";
      else this.inEsc = null;
      this.esc = "";
      return;
    }
    if (this.inEsc === "osc") {
      if (ch === "\x07" || ch === "\\") this.inEsc = null;
      return;
    }
    if (this.inEsc === "csi") {
      this.esc += ch;
      if (/[@-~]/.test(ch)) {
        this.csi(this.esc);
        this.inEsc = null;
      }
      return;
    }
    const i = this.lines.length - 1;
    switch (ch) {
      case "\x1b":
        this.inEsc = "esc";
        return;
      case "\n":
        this.lines.push("");
        this.col = 0;
        return;
      case "\r":
        this.col = 0;
        return;
      case "\b":
        this.col = Math.max(0, this.col - 1);
        return;
      case "\x07":
        return;
      default:
        if (ch < " " && ch !== "\t") return;
        this.lines[i] = this.setCol(this.lines[i], this.col, ch);
        this.col++;
    }
  }

  private csi(seq: string) {
    const final = seq[seq.length - 1];
    const n = Number.parseInt(seq, 10);
    const i = this.lines.length - 1;
    if (final === "K") {
      const mode = Number.isNaN(n) ? 0 : n;
      if (mode === 0) this.lines[i] = this.lines[i].slice(0, this.col);
      else if (mode === 2) this.lines[i] = "";
      else this.lines[i] = " ".repeat(this.col) + this.lines[i].slice(this.col);
    } else if (final === "J" && (n === 2 || n === 3)) {
      this.lines = [""];
      this.col = 0;
    } else if (final === "D") this.col = Math.max(0, this.col - (Number.isNaN(n) ? 1 : n));
    else if (final === "C") this.col += Number.isNaN(n) ? 1 : n;
    else if (final === "G") this.col = Math.max(0, (Number.isNaN(n) ? 1 : n) - 1);
    else if (final === "P") {
      const k = Number.isNaN(n) ? 1 : n;
      this.lines[i] = this.lines[i].slice(0, this.col) + this.lines[i].slice(this.col + k);
    }
  }

  text(): string {
    return this.lines.join("\n");
  }
}

/** Keys a phone keyboard can't send, as bytes. */
export const KEYS = {
  enter: "\r",
  tab: "\t",
  esc: "\x1b",
  ctrlC: "\x03",
  ctrlD: "\x04",
  up: "\x1b[A",
  down: "\x1b[B",
  right: "\x1b[C",
  left: "\x1b[D",
} as const;
