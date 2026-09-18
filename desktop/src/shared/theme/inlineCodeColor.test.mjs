import assert from "node:assert/strict";
import test from "node:test";

import { createThemeVars } from "./adaptive-theme.ts";
import {
  SYNTAX_THEMES,
  extractThemeInfo,
  loadThemeData,
} from "./theme-loader.ts";

const HSL_TRIPLE = /^-?[\d.]+ [\d.]+% [\d.]+%$/;
/** WCAG AA minimum for normal-size text. */
const AA_CONTRAST = 4.5;

function hslToRgb(triple) {
  const [h, s, l] = triple
    .split(" ")
    .map((part) => Number.parseFloat(part.replace("%", "")));
  const sat = s / 100;
  const light = l / 100;
  const k = (n) => (n + h / 30) % 12;
  const a = sat * Math.min(light, 1 - light);
  const f = (n) => light - a * Math.max(-1, Math.min(k(n) - 3, 9 - k(n), 1));
  return [f(0), f(8), f(4)];
}

function relativeLuminance([r, g, b]) {
  const [rs, gs, bs] = [r, g, b].map((channel) =>
    channel <= 0.03928 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4,
  );
  return 0.2126 * rs + 0.7152 * gs + 0.0722 * bs;
}

function contrastRatio(a, b) {
  const [lighter, darker] = [relativeLuminance(a), relativeLuminance(b)].sort(
    (x, y) => y - x,
  );
  return (lighter + 0.05) / (darker + 0.05);
}

async function themeVars(name) {
  const info = extractThemeInfo(name, await loadThemeData(name));
  const { vars } = createThemeVars(
    info.bg,
    info.fg,
    info.comment,
    { added: info.added, deleted: info.deleted, modified: info.modified },
    info.keyword,
  );
  return { info, vars };
}

test("every bundled theme yields an inline code color that clears AA on the chip", async () => {
  assert.ok(
    SYNTAX_THEMES.length > 0,
    "the bundled theme audit cannot be empty",
  );

  for (const name of SYNTAX_THEMES) {
    const { vars } = await themeVars(name);
    const code = vars["--code-foreground"];

    assert.match(
      code,
      HSL_TRIPLE,
      `${name}: --code-foreground must be an HSL triple, got "${code}"`,
    );

    // Inline code chips paint their background with --muted, so that — not the
    // page background — is the surface the text has to stay legible against.
    const ratio = contrastRatio(hslToRgb(code), hslToRgb(vars["--muted"]));
    assert.ok(
      ratio >= AA_CONTRAST,
      `${name}: inline code contrast ${ratio.toFixed(2)} is below AA (${AA_CONTRAST})`,
    );
  }
});

test("inline code borrows the theme's keyword accent, not the body text color", async () => {
  const { info, vars } = await themeVars("github-light");

  // GitHub Light styles keywords crimson; that is the accent inline code should
  // pick up rather than falling back to the near-black editor foreground.
  assert.equal(info.keyword.toLowerCase(), "#d73a49");
  assert.notEqual(vars["--code-foreground"], vars["--foreground"]);
});

test("a theme whose keyword accent already clears AA is used verbatim", async () => {
  const { info, vars } = await themeVars("slack-ochin");

  assert.equal(info.keyword.toLowerCase(), "#7b30d0");
  // Already 5.8:1 on the chip — no nudging needed, so the hue survives intact.
  assert.equal(vars["--code-foreground"], "268.1 62.99% 50.2%");
});
