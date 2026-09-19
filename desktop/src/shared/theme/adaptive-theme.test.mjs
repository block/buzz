import assert from "node:assert/strict";
import test from "node:test";

import {
  createThemeVars,
  ensureContrast,
  luminance,
} from "./adaptive-theme.ts";

const MIN_RATIO = 4.5;

function contrast(hex1, hex2) {
  const l1 = luminance(hex1);
  const l2 = luminance(hex2);
  const [lighter, darker] = l1 >= l2 ? [l1, l2] : [l2, l1];
  return (lighter + 0.05) / (darker + 0.05);
}

function channels(hex) {
  return [
    parseInt(hex.slice(1, 3), 16),
    parseInt(hex.slice(3, 5), 16),
    parseInt(hex.slice(5, 7), 16),
  ];
}

// ── ensureContrast ──────────────────────────────────────────────────────────

test("ensureContrast darkens a pale color on a light surface", () => {
  const result = ensureContrast("#ffdce0", "#ffffff");
  assert.ok(contrast(result, "#ffffff") >= MIN_RATIO);
});

test("ensureContrast lightens a dim color on a dark surface", () => {
  const result = ensureContrast("#3d1518", "#0d1117");
  assert.ok(contrast(result, "#0d1117") >= MIN_RATIO);
});

test("ensureContrast leaves an already-compliant color unchanged", () => {
  assert.equal(ensureContrast("#8b0000", "#ffffff"), "#8b0000");
});

test("ensureContrast preserves the dominant channel", () => {
  const [r, g, b] = channels(ensureContrast("#ffdce0", "#ffffff"));
  assert.ok(r > g && r > b, "guarded red should stay red-dominant");
});

// ── createThemeVars destructive/status colors ───────────────────────────────

// Themes frequently define git-deleted only as a pale diff-gutter tint. Fed
// straight into --destructive, that made "Delete" actions nearly invisible in
// light mode. The status vars are raw hex, so assert the guard through them;
// the raw syntax bg is a conservative bound for the popover surface the guard
// actually targets (elevation shifts the popover toward the text color).
test("pale git-deleted tint becomes readable in a light theme", () => {
  const { vars } = createThemeVars("#ffffff", "#1f2328", "#6e7781", {
    added: "#e6ffec",
    deleted: "#ffdce0",
    modified: "#fff8c5",
  });
  assert.ok(contrast(vars["--status-deleted"], "#ffffff") >= MIN_RATIO);
  assert.ok(contrast(vars["--status-added"], "#ffffff") >= MIN_RATIO);
});

test("dim git-deleted tint becomes readable in a dark theme", () => {
  const { vars } = createThemeVars("#0d1117", "#e6edf3", "#8b949e", {
    added: "#12261e",
    deleted: "#25171c",
    modified: "#272115",
  });
  assert.ok(contrast(vars["--status-deleted"], "#0d1117") >= MIN_RATIO);
  assert.ok(contrast(vars["--status-added"], "#0d1117") >= MIN_RATIO);
});

test("fallback accents meet the ratio when the theme has no git colors", () => {
  for (const [bg, fg, comment] of [
    ["#ffffff", "#1f2328", "#6e7781"],
    ["#0d1117", "#e6edf3", "#8b949e"],
  ]) {
    const { vars } = createThemeVars(bg, fg, comment);
    assert.ok(contrast(vars["--status-deleted"], bg) >= MIN_RATIO);
    assert.ok(contrast(vars["--status-added"], bg) >= MIN_RATIO);
  }
});
