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

export type TerminalRow = {
  line: number;
  wrapped: boolean;
  spans: readonly TerminalSpan[];
};
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

export type TerminalSelectionRow = {
  boundaries: readonly number[];
  line: number;
  text: string;
  wrapped: boolean;
};

type RetainedRow = {
  wrapped: boolean;
  spans: readonly TerminalSpan[];
};

export type PaintContext = Pick<
  CanvasRenderingContext2D,
  | "fillStyle"
  | "font"
  | "textBaseline"
  | "fillRect"
  | "fillText"
  | "save"
  | "restore"
  | "globalAlpha"
>;

const COLOR_KIND = 0xff00_0000;
const COLOR_VALUE = 0x00ff_ffff;
const NAMED = 0x0100_0000;
const INDEXED = 0x0200_0000;
const RGB = 0x0300_0000;
const BOLD = 1 << 0;
// alacritty Flags::DIM (bit 7). Apps set this with SGR 2 for ghost text.
// Ghostty paints that text at faint-opacity 0.5 (foreground alpha only).
const DIM = 1 << 7;
const FAINT_OPACITY = 0.5;

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
    this.#rows = Array.from({ length: viewport.screenLines }, () => ({
      wrapped: false,
      spans: [],
    }));
    this.markAllDirty();
  }

  get viewport(): TerminalViewport {
    return this.#viewport;
  }

  selectionRows(): readonly TerminalSelectionRow[] {
    return this.#rows.map((row, line) => {
      const cells = Array.from({ length: this.#viewport.columns }, () => " ");
      for (const span of row.spans) {
        for (const cluster of span.clusters) {
          cells[cluster.column] = cluster.text;
          for (let offset = 1; offset < cluster.width; offset++) {
            cells[cluster.column + offset] = "";
          }
        }
      }
      const text = cells.join("");
      const retainedText = row.wrapped ? text : text.trimEnd();
      const boundaries = [0];
      for (const segment of new Intl.Segmenter(undefined, {
        granularity: "grapheme",
      }).segment(retainedText)) {
        boundaries.push(segment.index + segment.segment.length);
      }
      return {
        boundaries,
        line,
        text: retainedText,
        wrapped: row.wrapped,
      };
    });
  }

  /** One string per cell of a row ("" for the trailing cells of a wide character), `columns` long. */
  #cellsOf(line: number): string[] {
    const cells = Array.from({ length: this.#viewport.columns }, () => " ");
    for (const span of this.#rows[line]?.spans ?? []) {
      for (const cluster of span.clusters) {
        cells[cluster.column] = cluster.text;
        for (let offset = 1; offset < cluster.width; offset++) {
          cells[cluster.column + offset] = "";
        }
      }
    }
    return cells;
  }

  /**
   * Text between two cells (inclusive, 0-based row/column, either order), as a touch selection needs it: columns are cells,
   * not string offsets, so wide characters select correctly. Rows end with a newline unless they wrap into the next.
   */
  textBetweenCells(
    rowA: number,
    colA: number,
    rowB: number,
    colB: number,
    pane?: { start: number; end: number },
  ): string {
    const forward = rowA < rowB || (rowA === rowB && colA <= colB);
    const [r1, c1, r2, c2] = forward
      ? [rowA, colA, rowB, colB]
      : [rowB, colB, rowA, colA];
    const first = Math.max(0, r1);
    const last = Math.min(this.#rows.length - 1, r2);
    let out = "";
    for (let line = first; line <= last; line++) {
      const cells = this.#cellsOf(line);
      const lo = pane ? Math.max(0, pane.start) : 0;
      const hi = pane ? Math.min(cells.length - 1, pane.end) : cells.length - 1;
      const from = line === r1 ? Math.max(lo, c1) : lo;
      const to = line === r2 ? Math.min(hi, c2) : hi;
      let text = from <= to ? cells.slice(from, to + 1).join("") : "";
      // Soft-wrap only joins full-width rows; a pane-clipped row always ends its line.
      const wrapped = pane ? false : (this.#rows[line]?.wrapped ?? false);
      if (!wrapped || line === last) text = text.trimEnd();
      out += text;
      if (line < last && !wrapped) out += "\n";
    }
    return out;
  }

  /**
   * The pane (inclusive column span) containing `col`, for TUIs that split the screen with vertical borders (Herder's
   * spaces/agents list beside its workspace, tmux splits…). A column is a border when most rows draw a vertical line
   * there; the pane runs between the nearest borders on each side. No borders → the full width.
   */
  paneAt(col: number): { start: number; end: number } {
    const columns = this.#viewport.columns;
    const rows = this.#rows.length;
    const full = { start: 0, end: Math.max(0, columns - 1) };
    if (rows < 3 || columns < 3) return full;
    const counts = new Array<number>(columns).fill(0);
    for (let line = 0; line < rows; line++) {
      const cells = this.#cellsOf(line);
      for (let c = 0; c < columns; c++) if (PANE_BORDER.has(cells[c] ?? "")) counts[c]++;
    }
    const isBorder = (c: number) => counts[c] >= Math.max(3, Math.ceil(rows * 0.5));
    const at = Math.max(0, Math.min(columns - 1, col));
    if (isBorder(at)) return full;
    let start = at;
    while (start > 0 && !isBorder(start - 1)) start--;
    let end = at;
    while (end < columns - 1 && !isBorder(end + 1)) end++;
    return { start, end };
  }

  /** The run of non-blank cells around a cell (for double-tap / long-press word selection); null on blank space. */
  wordAt(row: number, col: number): { start: number; end: number } | null {
    if (row < 0 || row >= this.#rows.length) return null;
    const cells = this.#cellsOf(row);
    const blank = (i: number) => cells[i] === " ";
    if (col < 0 || col >= cells.length || blank(col)) return null;
    let start = col;
    let end = col;
    while (start > 0 && !blank(start - 1)) start--;
    while (end < cells.length - 1 && !blank(end + 1)) end++;
    return { start, end };
  }

  normalizeSelectionOffset(
    rowIndex: number,
    offset: number,
    edge: "start" | "end",
  ): number {
    const row = this.selectionRows()[rowIndex];
    if (!row) return 0;
    const clamped = Math.max(0, Math.min(offset, row.text.length));
    if (edge === "start") {
      for (let index = row.boundaries.length - 1; index >= 0; index--) {
        if (row.boundaries[index] <= clamped) return row.boundaries[index];
      }
      return 0;
    }
    return (
      row.boundaries.find((boundary) => boundary >= clamped) ?? row.text.length
    );
  }

  selectionText(
    startRow: number,
    startOffset: number,
    endRow: number,
    endOffset: number,
  ): string {
    const rows = this.selectionRows();
    if (
      startRow < 0 ||
      endRow < startRow ||
      endRow >= rows.length ||
      startOffset < 0 ||
      endOffset < 0
    ) {
      return "";
    }
    let selected = "";
    for (let index = startRow; index <= endRow; index++) {
      const row = rows[index];
      const from = index === startRow ? startOffset : 0;
      const to = index === endRow ? endOffset : row.text.length;
      selected += row.text.slice(from, to);
      if (index < endRow && !row.wrapped) selected += "\n";
    }
    return selected;
  }

  text(): string {
    return this.selectionRows()
      .map((row) => (row.wrapped ? row.text : `${row.text}\n`))
      .join("")
      .replace(/\n$/, "");
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
        this.#rows[row.line] = { wrapped: row.wrapped, spans: row.spans };
        this.#dirty.add(row.line);
      }
    }
    return true;
  }

  resize(viewport: TerminalViewport): void {
    this.#viewport = viewport;
    this.#rows = Array.from({ length: viewport.screenLines }, () => ({
      wrapped: false,
      spans: [],
    }));
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
      for (const span of this.#rows[line]?.spans ?? []) {
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
        const faint = (span.style.flags & DIM) !== 0;
        if (faint) context.globalAlpha = FAINT_OPACITY;
        for (const cluster of span.clusters) {
          context.fillText(
            cluster.text,
            cluster.column * metrics.width,
            y + metrics.baseline,
          );
        }
        if (faint) context.globalAlpha = 1;
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

/** Vertical box-drawing glyphs TUIs use as pane borders. */
const PANE_BORDER = new Set(["│", "┃", "║", "╎", "╏", "┆", "┇", "┊", "┋", "▏", "▕", "├", "┤", "┼", "┣", "┫", "╋", "╟", "╢", "╠", "╣", "╬"]);
