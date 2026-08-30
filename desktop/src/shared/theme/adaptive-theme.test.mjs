import assert from "node:assert/strict";
import test from "node:test";

import {
  contrastRatio,
  createThemeVars,
  hexToHsl,
  resolveDestructiveAccent,
} from "./adaptive-theme.ts";
import {
  SYNTAX_THEMES,
  extractThemeInfo,
  loadThemeData,
} from "./theme-loader.ts";

// The engine renders destructive text on the plain background, on the popover
// surface (background 80% / muted 20%) and on a focused dropdown row (popover
// 50% / muted 50%). Reconstruct those from the emitted vars to check the
// shipped values rather than a reimplementation of the derivation.
const MIN_SHIPPED_CONTRAST = 4.4;

function toHex({ r, g, b }) {
  const clamp = (n) => Math.max(0, Math.min(255, Math.round(n)));
  return `#${[r, g, b].map((c) => clamp(c).toString(16).padStart(2, "0")).join("")}`;
}

function hexToRgb(hex) {
  const long = /^#?([a-f\d]{2})([a-f\d]{2})([a-f\d]{2})$/i.exec(hex);
  if (long) {
    return {
      r: parseInt(long[1], 16),
      g: parseInt(long[2], 16),
      b: parseInt(long[3], 16),
    };
  }
  const short = /^#?([a-f\d])([a-f\d])([a-f\d])$/i.exec(hex);
  return {
    r: parseInt(short[1] + short[1], 16),
    g: parseInt(short[2] + short[2], 16),
    b: parseInt(short[3] + short[3], 16),
  };
}

function mixHex(hex1, hex2, factor) {
  const c1 = hexToRgb(hex1);
  const c2 = hexToRgb(hex2);
  return toHex({
    r: c1.r + (c2.r - c1.r) * factor,
    g: c1.g + (c2.g - c1.g) * factor,
    b: c1.b + (c2.b - c1.b) * factor,
  });
}

/** Parse the "H S% L%" component form emitted by hexToHsl back into hex. */
function hslVarToHex(value) {
  const [h, s, l] = value.split(" ").map((part) => Number.parseFloat(part));
  const sn = s / 100;
  const ln = l / 100;
  if (sn === 0) {
    const channel = ln * 255;
    return toHex({ r: channel, g: channel, b: channel });
  }
  const q = ln < 0.5 ? ln * (1 + sn) : ln + sn - ln * sn;
  const p = 2 * ln - q;
  const channelAt = (offset) => {
    let t = h / 360 + offset;
    if (t < 0) t += 1;
    if (t > 1) t -= 1;
    if (t < 1 / 6) return p + (q - p) * 6 * t;
    if (t < 1 / 2) return q;
    if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6;
    return p;
  };
  return toHex({
    r: channelAt(1 / 3) * 255,
    g: channelAt(0) * 255,
    b: channelAt(-1 / 3) * 255,
  });
}

function surfacesFromVars(vars) {
  const background = hslVarToHex(vars["--background"]);
  const muted = hslVarToHex(vars["--muted"]);
  const popover = mixHex(background, muted, 0.2);
  return [background, popover, mixHex(popover, muted, 0.5)];
}

function minContrast(color, surfaces) {
  return Math.min(...surfaces.map((surface) => contrastRatio(color, surface)));
}

test("hslVarToHex round-trips hexToHsl", () => {
  for (const hex of ["#ffffff", "#000000", "#808080", "#cf222e", "#f85149"]) {
    const roundTripped = hexToRgb(hslVarToHex(hexToHsl(hex)));
    const original = hexToRgb(hex);
    for (const channel of ["r", "g", "b"]) {
      assert.ok(
        Math.abs(roundTripped[channel] - original[channel]) <= 2,
        `${hex} channel ${channel}: ${roundTripped[channel]} vs ${original[channel]}`,
      );
    }
  }
});

test("contrastRatio matches the WCAG poles", () => {
  assert.ok(Math.abs(contrastRatio("#000000", "#ffffff") - 21) < 0.01);
  assert.ok(Math.abs(contrastRatio("#ffffff", "#000000") - 21) < 0.01);
  assert.equal(contrastRatio("#cf222e", "#cf222e"), 1);
});

test("resolveDestructiveAccent keeps a compliant theme accent", () => {
  const surfaces = ["#ffffff", "#fcfcfc", "#f6f6f6"];
  assert.equal(
    resolveDestructiveAccent("#a0111f", "#cf222e", surfaces),
    "#a0111f",
  );
});

test("resolveDestructiveAccent keeps an accent between the floor and AA", () => {
  // github-light's #d73a49 misses 4.5:1 on the focused-row surface but stays
  // well above the 3:1 readability floor — theme fidelity wins.
  const surfaces = ["#ffffff", "#fcfcfc", "#f6f6f6"];
  const worst = minContrast("#d73a49", surfaces);
  assert.ok(worst >= 3 && worst < 4.5, `precondition, got ${worst}`);
  assert.equal(
    resolveDestructiveAccent("#d73a49", "#cf222e", surfaces),
    "#d73a49",
  );
});

test("resolveDestructiveAccent substitutes the fallback for an unreadable accent", () => {
  // slack-ochin ships #FFF for gitDecoration.deletedResourceForeground on a
  // white background (issue #2725).
  const surfaces = ["#ffffff", "#fcfcfc", "#f6f6f6"];
  assert.ok(minContrast("#FFF", surfaces) < 3);
  assert.equal(
    resolveDestructiveAccent("#FFF", "#cf222e", surfaces),
    "#cf222e",
  );
});

test("resolveDestructiveAccent nudges when the fallback also misses", () => {
  const surfaces = ["#999999", "#a0a0a0", "#909090"];
  assert.ok(minContrast("#e5534b", surfaces) < 3);
  assert.ok(minContrast("#f85149", surfaces) < 4.5);

  const resolved = resolveDestructiveAccent("#e5534b", "#f85149", surfaces);
  assert.ok(
    minContrast(resolved, surfaces) >= 4.5,
    `resolved ${resolved} only reaches ${minContrast(resolved, surfaces)}`,
  );
  assert.notEqual(resolved, "#000000");

  const { r, g, b } = hexToRgb(resolved);
  assert.ok(r > g && r > b, `resolved ${resolved} lost its red hue`);
});

test("slack-ochin destructive text is readable", () => {
  // Real slack-ochin values via extractThemeInfo(loadThemeData("slack-ochin")).
  const { vars } = createThemeVars("#FFF", "#000", "#357b42", {
    added: "#ECB22E",
    deleted: "#FFF",
    modified: "#ECB22E",
  });

  const destructive = hslVarToHex(vars["--destructive"]);
  assert.notEqual(destructive, "#ffffff");
  assert.ok(
    minContrast(destructive, surfacesFromVars(vars)) >= MIN_SHIPPED_CONTRAST,
    `slack-ochin --destructive ${destructive} is unreadable`,
  );
});

test("every bundled theme emits readable destructive text", async () => {
  for (const name of SYNTAX_THEMES) {
    const info = extractThemeInfo(name, await loadThemeData(name));
    const { isDark, vars } = createThemeVars(info.bg, info.fg, info.comment, {
      added: info.added,
      deleted: info.deleted,
      modified: info.modified,
    });

    const surfaces = surfacesFromVars(vars);
    const themeAccent = info.deleted ?? (isDark ? "#f85149" : "#cf222e");
    const rawWorst = minContrast(themeAccent, surfaces);
    const destructive = hslVarToHex(vars["--destructive"]);
    const shipped = minContrast(destructive, surfaces);

    if (rawWorst >= 3.05) {
      // Above the keep floor: the theme's own accent survives verbatim.
      assert.equal(vars["--destructive"], hexToHsl(themeAccent), name);
    } else if (rawWorst < 2.95) {
      // Below the floor: the replacement must reach AA.
      assert.ok(
        shipped >= MIN_SHIPPED_CONTRAST,
        `${name}: --destructive ${destructive} reaches only ${shipped.toFixed(2)}`,
      );
    } else {
      // Within HSL-rounding distance of the floor: either branch is
      // acceptable, but the result must still read.
      assert.ok(shipped >= 2.9, name);
    }
  }
});

test("compliant theme accents stay byte-identical", async () => {
  // Both ship a deleted color that already clears the threshold and differs
  // from the curated fallback, so an unchanged output can only come from the
  // pass-through branch.
  for (const [name, deleted] of [
    ["gruvbox-light-hard", "#cc241d"],
    ["catppuccin-mocha", "#f38ba8"],
  ]) {
    const info = extractThemeInfo(name, await loadThemeData(name));
    assert.equal(info.deleted, deleted);

    const { vars } = createThemeVars(info.bg, info.fg, info.comment, {
      added: info.added,
      deleted: info.deleted,
      modified: info.modified,
    });
    assert.equal(vars["--destructive"], hexToHsl(deleted), name);
  }
});
