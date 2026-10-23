// Ported from Berd's agent-activity-panel (branch zmarley/agent-activity-panel).
// Small math/color/string helpers shared by the activity scene compiler and
// canvas renderer.

import type { Rgb } from "./activityScene";

export function clamp(value: number, min: number, max: number) {
  return Math.max(min, Math.min(max, value));
}

export function lerp(a: number, b: number, t: number) {
  return a + (b - a) * t;
}

export function smooth(value: number) {
  const t = clamp(value, 0, 1);
  return t * t * (3 - 2 * t);
}

export function basename(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts.at(-1) ?? path;
}

interface ActivityEventHashInput {
  kind: string;
  path?: string;
  intent?: string;
}

export function hashActivityEvents(
  events: readonly ActivityEventHashInput[],
): number {
  let hash = 0x811c9dc5;

  for (const event of events) {
    for (let index = 0; index < event.kind.length; index += 1) {
      hash ^= event.kind.charCodeAt(index);
      hash = Math.imul(hash, 0x01000193);
    }

    hash ^= 124;
    hash = Math.imul(hash, 0x01000193);

    const path = event.path ?? "";
    for (let index = 0; index < path.length; index += 1) {
      hash ^= path.charCodeAt(index);
      hash = Math.imul(hash, 0x01000193);
    }

    hash ^= 31;
    hash = Math.imul(hash, 0x01000193);

    // Narration can be patched onto an existing tool call after the fact;
    // fold it in so cached scenes don't serve stale intent.
    const intent = event.intent ?? "";
    for (let index = 0; index < intent.length; index += 1) {
      hash ^= intent.charCodeAt(index);
      hash = Math.imul(hash, 0x01000193);
    }

    hash ^= 17;
    hash = Math.imul(hash, 0x01000193);
  }

  return hash >>> 0;
}

export function hexToRgb(color: string): Rgb {
  const trimmed = color.trim();
  const hex = /^#?([0-9a-f]{6})$/i.exec(trimmed);
  if (hex) {
    const value = Number.parseInt(hex[1], 16);
    return [(value >> 16) & 255, (value >> 8) & 255, value & 255];
  }

  const shortHex = /^#?([0-9a-f]{3})$/i.exec(trimmed);
  if (shortHex) {
    const expanded = shortHex[1]
      .split("")
      .map((digit) => `${digit}${digit}`)
      .join("");
    const value = Number.parseInt(expanded, 16);
    return [(value >> 16) & 255, (value >> 8) & 255, value & 255];
  }

  return [255, 255, 255];
}

export function rgba(color: Rgb, alpha: number) {
  return `rgba(${color[0]},${color[1]},${color[2]},${alpha})`;
}

export function mixRgb(a: Rgb, b: Rgb, amount: number): Rgb {
  const t = clamp(amount, 0, 1);
  return [
    Math.round(lerp(a[0], b[0], t)),
    Math.round(lerp(a[1], b[1], t)),
    Math.round(lerp(a[2], b[2], t)),
  ];
}

export function binarySearchPrefix(
  times: readonly number[],
  t: number,
): number {
  let lo = 0;
  let hi = times.length;

  while (lo < hi) {
    const mid = Math.floor((lo + hi) / 2);
    if (times[mid] <= t) lo = mid + 1;
    else hi = mid;
  }

  return lo;
}
