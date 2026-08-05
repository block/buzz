import {
  ANSI_COLOR_NAMES,
  type AnsiColorName,
  type TerminalPalette,
} from "@/shared/theme/terminal-palette";

export type TerminalViewport = {
  generation: number;
  columns: number;
  screenLines: number;
};

export type TerminalStyle = { fg: number; bg: number; flags: number };

export type TerminalCluster = {
  column: number;
  text: string;
  width: 1 | 2;
};

export type TerminalSpan = {
  style: TerminalStyle;
  clusters: readonly TerminalCluster[];
};

export type TerminalRow = { line: number; spans: readonly TerminalSpan[] };
export type TerminalCursor = {
  line: number;
  column: number;
  visible: boolean;
};
export type TerminalFrame = {
  viewport: TerminalViewport;
  rows: readonly TerminalRow[];
  cursor: TerminalCursor;
  full: boolean;
};

export type CellMetrics = {
  width: number;
  height: number;
  baseline: number;
  font: string;
  boldFont: string;
};

export const TERMINAL_CELL_METRICS = {
  width: 8.4,
  height: 17,
  baseline: 13,
  font: '14px "JetBrains Mono", monospace',
  boldFont: '700 14px "JetBrains Mono", monospace',
} as const satisfies CellMetrics;

type RetainedRow = readonly TerminalSpan[];

export type PaintContext = Pick<
  CanvasRenderingContext2D,
  | "fillStyle"
  | "font"
  | "textBaseline"
  | "fillRect"
  | "fillText"
  | "save"
  | "restore"
>;

const COLOR_KIND = 0xff00_0000;
const COLOR_VALUE = 0x00ff_ffff;
const NAMED = 0x0100_0000;
const INDEXED = 0x0200_0000;
const RGB = 0x0300_0000;
const BOLD = 1 << 0;

function xtermIndexed(index: number, palette: TerminalPalette): string {
  if (index < 16) return palette.ansi[ANSI_COLOR_NAMES[index]];
  if (index < 232) {
    const offset = index - 16;
    const levels = [0, 95, 135, 175, 215, 255];
    const r = levels[Math.floor(offset / 36)];
    const g = levels[Math.floor((offset % 36) / 6)];
    const b = levels[offset % 6];
    return `rgb(${r} ${g} ${b})`;
  }
  const gray = 8 + (index - 232) * 10;
  return `rgb(${gray} ${gray} ${gray})`;
}

export function resolvePackedColor(
  packed: number,
  palette: TerminalPalette,
  fallback: "foreground" | "background",
): string {
  const kind = packed & COLOR_KIND;
  const value = packed & COLOR_VALUE;
  if (kind === RGB) {
    return `#${value.toString(16).padStart(6, "0")}`;
  }
  if (kind === INDEXED) return xtermIndexed(value & 0xff, palette);
  if (kind === NAMED) {
    if (value < 16) return palette.ansi[ANSI_COLOR_NAMES[value]];
    if (value === 256 || value === 272) return palette.foreground;
    if (value === 257) return palette.background;
    if (value === 258) return palette.cursor;
    if (value >= 259 && value <= 266) {
      return palette.ansi[ANSI_COLOR_NAMES[value - 259] as AnsiColorName];
    }
    if (value === 271) return palette.ansi.brightWhite;
  }
  return palette[fallback];
}

/** Retained terminal grid. Applying a frame only dirties rows it names. */
export class TerminalGrid {
  #viewport: TerminalViewport;
  #rows: RetainedRow[];
  #dirty = new Set<number>();
  #cursor: TerminalCursor = { line: 0, column: 0, visible: false };
  #cursorPainted = true;

  constructor(viewport: TerminalViewport) {
    this.#viewport = viewport;
    this.#rows = Array.from({ length: viewport.screenLines }, () => []);
    this.markAllDirty();
  }

  get viewport(): TerminalViewport {
    return this.#viewport;
  }

  apply(frame: TerminalFrame): boolean {
    if (
      frame.viewport.generation !== this.#viewport.generation ||
      frame.viewport.columns !== this.#viewport.columns ||
      frame.viewport.screenLines !== this.#viewport.screenLines
    ) {
      return false;
    }
    if (frame.full) this.markAllDirty();
    this.#dirty.add(this.#cursor.line);
    this.#cursor = frame.cursor;
    this.#dirty.add(this.#cursor.line);
    for (const row of frame.rows) {
      if (row.line < this.#rows.length) {
        this.#rows[row.line] = row.spans;
        this.#dirty.add(row.line);
      }
    }
    return true;
  }

  resize(viewport: TerminalViewport): void {
    this.#viewport = viewport;
    this.#rows = Array.from({ length: viewport.screenLines }, () => []);
    this.#cursor = { line: 0, column: 0, visible: false };
    this.markAllDirty();
  }

  markAllDirty(): void {
    for (let line = 0; line < this.#viewport.screenLines; line++) {
      this.#dirty.add(line);
    }
  }

  setCursorPainted(painted: boolean): void {
    if (this.#cursorPainted === painted) return;
    this.#cursorPainted = painted;
    this.#dirty.add(this.#cursor.line);
  }

  paint(
    context: PaintContext,
    metrics: CellMetrics,
    palette: TerminalPalette,
  ): number {
    const started = performance.now();
    context.save();
    context.textBaseline = "alphabetic";
    const dirty = [...this.#dirty].sort((a, b) => a - b);
    for (const line of dirty) {
      const y = line * metrics.height;
      context.fillStyle = palette.background;
      context.fillRect(
        0,
        y,
        this.#viewport.columns * metrics.width,
        metrics.height,
      );
      for (const span of this.#rows[line] ?? []) {
        const background = resolvePackedColor(
          span.style.bg,
          palette,
          "background",
        );
        context.fillStyle = background;
        for (const cluster of span.clusters) {
          context.fillRect(
            cluster.column * metrics.width,
            y,
            cluster.width * metrics.width,
            metrics.height,
          );
        }
        context.fillStyle = resolvePackedColor(
          span.style.fg,
          palette,
          "foreground",
        );
        context.font =
          span.style.flags & BOLD ? metrics.boldFont : metrics.font;
        for (const cluster of span.clusters) {
          context.fillText(
            cluster.text,
            cluster.column * metrics.width,
            y + metrics.baseline,
          );
        }
      }
    }
    if (this.#cursor.visible && this.#cursorPainted) {
      context.fillStyle = palette.cursor;
      context.fillRect(
        this.#cursor.column * metrics.width,
        this.#cursor.line * metrics.height,
        Math.max(1, metrics.width * 0.12),
        metrics.height,
      );
    }
    context.restore();
    this.#dirty.clear();
    return performance.now() - started;
  }
}
