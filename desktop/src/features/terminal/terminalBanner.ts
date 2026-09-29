export type TerminalBannerLayer = "field" | "head" | "bevel_hi" | "bevel_lo";

export type TerminalBannerCell = Readonly<{
  char: string;
  layer?: TerminalBannerLayer;
  t: number;
}>;

export type TerminalBanner = Readonly<{
  cells: readonly (readonly TerminalBannerCell[])[];
}>;

const HEX = [
  [0, 1, "_"],
  [0, 2, "_"],
  [1, 0, "/"],
  [1, 3, "\\"],
  [2, 0, "\\"],
  [2, 1, "_"],
  [2, 2, "_"],
  [2, 3, "/"],
] as const;

const GLYPHS: Readonly<Record<string, readonly string[]>> = {
  b: ["██     ", "██▄▄▄  ", "██▀▀██ ", "██  ██ ", "██████ "],
  u: ["       ", "██  ██ ", "██  ██ ", "██  ██ ", "▀█████ "],
  z: ["       ", "██████ ", "   ▄██ ", " ▄██▀  ", "██████ "],
  t: [" ██    ", "█████  ", " ██    ", " ██    ", "  ███  "],
  e: ["       ", " ▄███▄ ", "██▄▄▄█ ", "██     ", " ▀███▀ "],
  r: ["       ", "██ ▄██ ", "███▀▀  ", "██     ", "██     "],
  m: ["        ", "██▄██▄██", "██ ██ ██", "██ ██ ██", "██ ██ ██"],
  " ": ["   ", "   ", "   ", "   ", "   "],
};

const INK_FRAME = {
  topLeft: "▛",
  topRight: "▜",
  bottomLeft: "▙",
  bottomRight: "▟",
  top: "▀",
  bottom: "▄",
  left: "▌",
  right: "▐",
} as const;
const LAYERS: readonly TerminalBannerLayer[] = [
  "field",
  "head",
  "bevel_hi",
  "bevel_lo",
];
type TerminalBannerEmitter =
  | "field"
  | "top_row"
  | "left_rail"
  | "right_rail"
  | "bottom_row"
  | "wordmark";
const EMITTERS: readonly TerminalBannerEmitter[] = [
  "field",
  "top_row",
  "left_rail",
  "right_rail",
  "bottom_row",
  "wordmark",
];

function trimRight(value: string): string {
  return value.replace(/\s+$/, "");
}

function wordmark(gap: number): readonly string[] {
  const rows = Array.from({ length: 5 }, () => "");
  for (const [index, letter] of [..."buzz term"].entries()) {
    const glyph = GLYPHS[letter];
    for (let row = 0; row < rows.length; row += 1) {
      rows[row] += glyph[row] + (index < 8 ? " ".repeat(gap) : "");
    }
  }
  return rows.map(trimRight).filter((row) => row.trim());
}

function fitWordmark(columns: number): readonly string[] | null {
  for (const gap of [3, 2, 1, 0]) {
    const mark = wordmark(gap);
    if (Math.max(...mark.map((row) => row.length)) <= columns) return mark;
  }
  return null;
}

function stableRandom(band: number, index: number): number {
  let value = 2166136261;
  for (const code of [...`${band}:${index}`].map((char) =>
    char.charCodeAt(0),
  )) {
    value = Math.imul(value ^ code, 16777619);
  }
  return (value >>> 0) / 0xffffffff;
}

function completeHexes(columns: number, rows: number) {
  const cells: { band: number; index: number; y: number; x: number }[] = [];
  for (let band = 0, y = -2; y < rows; band += 1, y += 2) {
    for (
      let index = 0, x = -6 + (band % 2 ? 3 : 0);
      x < columns;
      index += 1, x += 6
    ) {
      if (
        HEX.every(
          ([dy, dx]) =>
            y + dy >= 0 && y + dy < rows && x + dx >= 0 && x + dx < columns,
        )
      ) {
        cells.push({ band, index, y, x });
      }
    }
  }
  return cells;
}

export function buildTerminalBanner(
  columns: number,
  rows: number,
  cellAspect: number,
): TerminalBanner | null {
  const mark = fitWordmark(columns - 10);
  if (!mark || rows < mark.length + 6) return null;

  const width = Math.max(...mark.map((row) => row.length));
  const height = mark.length;
  const paddingX = columns >= width + 24 ? 6 : 2;
  const paddingY = rows >= height + 12 ? 2 : 1;
  const innerWidth = Math.min(columns - 4, width + paddingX * 2);
  const frameWidth = innerWidth + 2;
  const frameHeight = height + paddingY * 2 + 2;
  const left = Math.floor((columns - frameWidth) / 2);
  const top = Math.floor((rows - frameHeight) / 2);
  const centerX = left + frameWidth / 2;
  const centerY = top + frameHeight / 2;
  const grid: TerminalBannerCell[][] = Array.from({ length: rows }, () =>
    Array.from({ length: columns }, () => ({ char: " ", t: 0 })),
  );
  const emitted = new Set<TerminalBannerEmitter>();
  const put = (
    y: number,
    x: number,
    char: string,
    layer: TerminalBannerLayer,
    t = 0,
    emitter?: TerminalBannerEmitter,
  ) => {
    if (y >= 0 && y < rows && x >= 0 && x < columns) {
      grid[y][x] = { char, layer, t };
      if (emitter) emitted.add(emitter);
    }
  };

  const halfWidth = frameWidth / 2;
  const halfHeight = (frameHeight / 2) * cellAspect;
  const viewportHalfWidth = columns / 2;
  const viewportHalfHeight = (rows / 2) * cellAspect;
  for (const hex of completeHexes(columns, rows)) {
    if (
      HEX.some(
        ([offsetY, offsetX]) =>
          hex.y + offsetY >= top - 1 &&
          hex.y + offsetY < top + frameHeight + 1 &&
          hex.x + offsetX >= left - 2 &&
          hex.x + offsetX < left + frameWidth + 2,
      )
    ) {
      continue;
    }
    const dx = hex.x + 1.5 - centerX;
    const dy = (hex.y + 1 - centerY) * cellAspect;
    const ellipse = Math.hypot(dx / halfWidth, dy / halfHeight);
    if (ellipse <= 1) continue;
    const viewport = Math.max(
      Math.abs(dx) / viewportHalfWidth,
      Math.abs(dy) / viewportHalfHeight,
    );
    const ray =
      ellipse <= viewport
        ? 0
        : Math.min(
            1,
            Math.max(0, (viewport * (ellipse - 1)) / (ellipse - viewport)),
          );
    if (
      ray > 0.55 &&
      stableRandom(hex.band, hex.index) < ((ray - 0.55) / 0.45) ** 1.3
    )
      continue;
    for (const [offsetY, offsetX, char] of HEX)
      put(hex.y + offsetY, hex.x + offsetX, char, "field", ray * ray, "field");
  }

  for (
    let y = Math.max(0, top - 1);
    y < Math.min(rows, top + frameHeight + 1);
    y += 1
  ) {
    for (
      let x = Math.max(0, left - 2);
      x < Math.min(columns, left + frameWidth + 2);
      x += 1
    )
      grid[y][x] = { char: " ", t: 0 };
  }

  const frame = INK_FRAME;
  put(top, left, frame.topLeft, "bevel_hi", 0, "top_row");
  for (let x = 0; x < innerWidth; x += 1)
    put(top, left + x + 1, frame.top, "bevel_hi", 0, "top_row");
  put(top, left + frameWidth - 1, frame.topRight, "bevel_hi", 0, "top_row");
  for (let y = top + 1; y < top + frameHeight - 1; y += 1) {
    put(y, left, frame.left, "bevel_hi", 0, "left_rail");
    put(y, left + frameWidth - 1, frame.right, "bevel_lo", 0, "right_rail");
  }
  put(
    top + frameHeight - 1,
    left,
    frame.bottomLeft,
    "bevel_lo",
    0,
    "bottom_row",
  );
  for (let x = 0; x < innerWidth; x += 1)
    put(
      top + frameHeight - 1,
      left + x + 1,
      frame.bottom,
      "bevel_lo",
      0,
      "bottom_row",
    );
  put(
    top + frameHeight - 1,
    left + frameWidth - 1,
    frame.bottomRight,
    "bevel_lo",
    0,
    "bottom_row",
  );

  const markLeft = left + Math.floor((frameWidth - width) / 2);
  const markTop = top + paddingY + 1;
  const ink = mark.flatMap((row, y) =>
    [...row].flatMap((char, x) => (char === " " ? [] : [{ char, x, y }])),
  );
  if (ink.length === 0) return null;
  const minX = Math.min(...ink.map(({ x }) => x));
  const maxX = Math.max(...ink.map(({ x }) => x));
  const span = Math.max(1, maxX - minX);
  for (const cell of ink)
    put(
      markTop + cell.y,
      markLeft + cell.x,
      cell.char,
      "head",
      (cell.x - minX) / span,
      "wordmark",
    );

  const seen = new Map<TerminalBannerLayer, number[]>();
  for (const row of grid)
    for (const cell of row)
      if (cell.layer)
        seen.set(cell.layer, [...(seen.get(cell.layer) ?? []), cell.t]);
  if (LAYERS.some((layer) => !seen.has(layer))) return null;
  if (EMITTERS.some((emitter) => !emitted.has(emitter))) return null;
  const sweep = seen.get("head") ?? [];
  if (Math.min(...sweep) !== 0 || Math.max(...sweep) !== 1) return null;

  return { cells: grid };
}
