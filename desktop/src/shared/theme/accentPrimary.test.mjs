import assert from "node:assert/strict";
import test from "node:test";
import { hexToHsl } from "./adaptive-theme.ts";
import { ACCENT_COLORS, accentPrimary } from "./ThemeProvider.tsx";

function luminance(hex) {
  const channels = [1, 3, 5].map((i) => {
    const value = parseInt(hex.slice(i, i + 2), 16) / 255;
    return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
}

function contrast(a, b) {
  const [high, low] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (high + 0.05) / (low + 0.05);
}

// Light backgrounds down to the adaptive engine's 0.5-luminance light cutoff.
const LIGHT = ["#ffffff", "#fafafa", "#f6f8fa", "#eff1f5", "#bcbcbc"];
const DARK = ["#0d1117", "#24273a", "#1e1e1e"];

test("Chartreuse reads as text and as a filled control on every theme", () => {
  for (const background of [...LIGHT, ...DARK]) {
    const { fill, ink } = accentPrimary("#d7d72e", hexToHsl(background));
    assert.ok(contrast(fill, background) >= 4.5, `text on ${background}`);
    assert.ok(contrast(ink, fill) >= 4.5, `ink on fill over ${background}`);
  }
  // Dark themes keep the raw accent.
  for (const background of DARK) {
    assert.equal(
      accentPrimary("#d7d72e", hexToHsl(background)).fill,
      "#d7d72e",
    );
  }
});

test("legacy accents keep their established fill", () => {
  for (const { value } of ACCENT_COLORS) {
    if (value === "neutral" || value === "#d7d72e") continue;
    for (const background of [...LIGHT, ...DARK]) {
      assert.equal(accentPrimary(value, hexToHsl(background)).fill, value);
    }
  }
});
