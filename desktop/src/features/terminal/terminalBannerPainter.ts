import type { TerminalPalette } from "../../shared/theme/terminal-palette";
import type { TerminalBanner, TerminalBannerLayer } from "./terminalBanner";
import { bannerColor, bannerStops } from "./terminalBannerColor";
import {
  type BannerColorTable,
  type BannerWaves,
  makeCellHue,
} from "./terminalBannerWave";
import { TERMINAL_CELL_METRICS } from "./terminalRenderer";

/**
 * Optional animation input. ABSENT means the shipped static banner, not a
 * stopped animation — those differ, because a halted rAF loop still parks on
 * whatever phase it stopped at. `prefers-reduced-motion` takes this path, and
 * the painter then calls `bannerColor` with no hue argument, which is the exact
 * coupled call the static banner shipped with. Verified as byte equality on
 * every draw call, not asserted by inspection.
 */
export type TerminalBannerMotion = {
  phase: { field: number; wordmark: number };
  table: BannerColorTable;
  /** Tuning override. Omitted = the shipped FIELD_WAVE/WORDMARK_WAVE constants;
   *  supplied only by the gates that drive each knob to its documented extreme. */
  waves?: BannerWaves;
};

export function paintTerminalBanner(
  canvas: HTMLCanvasElement,
  banner: TerminalBanner,
  palette: TerminalPalette,
  dpr: number,
  motion?: TerminalBannerMotion,
): boolean {
  const bounds = canvas.getBoundingClientRect();
  const pixelWidth = Math.max(1, Math.round(bounds.width * dpr));
  const pixelHeight = Math.max(1, Math.round(bounds.height * dpr));
  if (canvas.width !== pixelWidth) canvas.width = pixelWidth;
  if (canvas.height !== pixelHeight) canvas.height = pixelHeight;

  const context = canvas.getContext("2d");
  if (!context) return false;
  context.setTransform(dpr, 0, 0, dpr, 0, 0);
  context.clearRect(0, 0, bounds.width, bounds.height);
  // The overlay sits above the grid canvas, so it needs its own opaque
  // ground: without one the shell output painted underneath shows through
  // the gaps between banner glyphs.
  context.fillStyle = palette.background;
  context.fillRect(0, 0, bounds.width, bounds.height);
  context.font = TERMINAL_CELL_METRICS.font;
  context.textBaseline = "alphabetic";

  // Resolve the colour strategy ONCE, outside the loop: the motion and static
  // paths differ in what they close over, not per cell. The field wave is a
  // projection onto a direction vector, so the hue closure needs the banner's
  // extent in BOTH axes; it is built here so the per-cell path stays four
  // scalars with no per-cell allocation.
  const columns = banner.cells[0]?.length ?? 1;
  const rows = banner.cells.length || 1;
  const colourOf = motion
    ? (() => {
        const hueAt = makeCellHue(columns, rows, motion.phase, motion.waves);
        return (
          layer: TerminalBannerLayer,
          t: number,
          column: number,
          row: number,
        ) => motion.table.lookup(layer, t, hueAt(layer, t, column, row));
      })()
    : (() => {
        const { stops } = bannerStops(palette);
        return (layer: TerminalBannerLayer, t: number) =>
          bannerColor(palette, stops, layer, t);
      })();

  for (const [row, cells] of banner.cells.entries()) {
    for (const [column, cell] of cells.entries()) {
      if (cell.char === " " || !cell.layer) continue;
      context.fillStyle = colourOf(cell.layer, cell.t, column, row);
      context.fillText(
        cell.char,
        column * TERMINAL_CELL_METRICS.width,
        row * TERMINAL_CELL_METRICS.height + TERMINAL_CELL_METRICS.baseline,
      );
    }
  }
  return true;
}
