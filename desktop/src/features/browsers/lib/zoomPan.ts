// Pure zoom/pan math for the full-screen screenshot view. Coordinates are relative to the stage centre; the image is drawn
// centred, scaled by `scale`, then translated by (x, y).
export type View = { scale: number; x: number; y: number };
export type Size = { w: number; h: number };

export const MIN_SCALE = 1;
export const MAX_SCALE = 8;
export const DOUBLE_TAP_SCALE = 2.5;
export const IDENTITY: View = { scale: 1, x: 0, y: 0 };

export const clampScale = (s: number) => Math.min(MAX_SCALE, Math.max(MIN_SCALE, s));

/** Base size of the image: its height fills the stage, width follows the aspect ratio (so it may be wider than the stage). */
export function baseSize(stage: Size, aspect: number): Size {
  return { w: stage.h * aspect, h: stage.h };
}

/** How far the scaled image may be dragged from centre on each axis (0 when it fits inside the stage). */
export function maxPan(base: Size, stage: Size, scale: number): { x: number; y: number } {
  return { x: Math.max(0, (base.w * scale - stage.w) / 2), y: Math.max(0, (base.h * scale - stage.h) / 2) };
}

export function clampView(v: View, base: Size, stage: Size): View {
  const scale = clampScale(v.scale);
  const m = maxPan(base, stage, scale);
  return { scale, x: Math.min(m.x, Math.max(-m.x, v.x)), y: Math.min(m.y, Math.max(-m.y, v.y)) };
}

/** Zoom by `factor` keeping the image point under `at` (stage-centre coordinates) fixed. */
export function zoomAt(v: View, factor: number, at: { x: number; y: number }, base: Size, stage: Size): View {
  const scale = clampScale(v.scale * factor);
  const k = scale / v.scale;
  return clampView({ scale, x: at.x - (at.x - v.x) * k, y: at.y - (at.y - v.y) * k }, base, stage);
}

/** Double-tap / double-click: back to 1x when zoomed in, otherwise zoom in at the tapped point. */
export function toggleZoom(v: View, at: { x: number; y: number }, base: Size, stage: Size): View {
  if (v.scale > 1.05) return IDENTITY;
  return zoomAt(v, DOUBLE_TAP_SCALE, at, base, stage);
}

export const panBy = (v: View, dx: number, dy: number, base: Size, stage: Size): View =>
  clampView({ ...v, x: v.x + dx, y: v.y + dy }, base, stage);

/** Wheel / trackpad: exponential so small deltas feel smooth. */
export const wheelFactor = (deltaY: number, deltaMode = 0) => Math.exp(-deltaY * (deltaMode === 1 ? 0.05 : 0.0025));
