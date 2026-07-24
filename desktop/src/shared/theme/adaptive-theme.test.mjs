import assert from "node:assert/strict";
import test from "node:test";

import { createThemeVars, luminance } from "./adaptive-theme.ts";

const MINIMUM_TEXT_CONTRAST = 4.5;

function hslComponentsToHex(value) {
  const match = /^(-?[\d.]+) ([\d.]+)% ([\d.]+)%$/.exec(value);
  assert.ok(match, `expected HSL components, received ${value}`);

  const hue = (((Number(match[1]) % 360) + 360) % 360) / 360;
  const saturation = Number(match[2]) / 100;
  const lightness = Number(match[3]) / 100;

  const hueToRgb = (p, q, channel) => {
    let normalized = channel;
    if (normalized < 0) normalized += 1;
    if (normalized > 1) normalized -= 1;
    if (normalized < 1 / 6) return p + (q - p) * 6 * normalized;
    if (normalized < 1 / 2) return q;
    if (normalized < 2 / 3) return p + (q - p) * (2 / 3 - normalized) * 6;
    return p;
  };

  const channels =
    saturation === 0
      ? [lightness, lightness, lightness]
      : (() => {
          const q =
            lightness < 0.5
              ? lightness * (1 + saturation)
              : lightness + saturation - lightness * saturation;
          const p = 2 * lightness - q;
          return [
            hueToRgb(p, q, hue + 1 / 3),
            hueToRgb(p, q, hue),
            hueToRgb(p, q, hue - 1 / 3),
          ];
        })();

  return `#${channels
    .map((channel) =>
      Math.round(channel * 255)
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")}`;
}

function contrastRatio(first, second) {
  const firstLuminance = luminance(first);
  const secondLuminance = luminance(second);
  const lighter = Math.max(firstLuminance, secondLuminance);
  const darker = Math.min(firstLuminance, secondLuminance);
  return (lighter + 0.05) / (darker + 0.05);
}

function mix(first, second, factor) {
  const channels = (hex) =>
    [1, 3, 5].map((offset) =>
      Number.parseInt(hex.slice(offset, offset + 2), 16),
    );
  const firstChannels = channels(first);
  const secondChannels = channels(second);

  return `#${firstChannels
    .map((channel, index) =>
      Math.round(channel + (secondChannels[index] - channel) * factor)
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")}`;
}

function lightThemeVars(deleted) {
  return createThemeVars("#ffffff", "#1f2328", "#6e7781", {
    added: null,
    deleted,
    modified: null,
  }).vars;
}

function menuSurface(vars) {
  return mix(
    hslComponentsToHex(vars["--background"]),
    hslComponentsToHex(vars["--muted"]),
    0.2,
  );
}

test("destructive text meets WCAG AA contrast in GitHub Light", () => {
  const vars = lightThemeVars("#d73a49");

  assert.ok(
    contrastRatio(vars["--status-deleted"], menuSurface(vars)) >=
      MINIMUM_TEXT_CONTRAST,
  );
});

test("pale deleted-background tints become readable destructive text", () => {
  const vars = lightThemeVars("#ffdce0");

  assert.ok(
    contrastRatio(vars["--status-deleted"], menuSurface(vars)) >=
      MINIMUM_TEXT_CONTRAST,
  );
});

test("already-compliant destructive colors remain unchanged", () => {
  const vars = lightThemeVars("#8b0000");

  assert.equal(vars["--status-deleted"], "#8b0000");
});
