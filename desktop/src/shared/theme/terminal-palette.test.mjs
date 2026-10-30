import assert from "node:assert/strict";
import test from "node:test";

import {
  SYNTAX_THEMES,
  extractThemeInfo,
  loadThemeData,
} from "./theme-loader.ts";
import { ANSI_COLOR_NAMES } from "./terminal-palette.ts";

const HEX = /^#[\da-f]{6}$/;

test("every bundled theme produces a complete normalized terminal palette", async () => {
  assert.ok(
    SYNTAX_THEMES.length > 0,
    "the bundled theme audit cannot be empty",
  );

  for (const name of SYNTAX_THEMES) {
    const theme = await loadThemeData(name);
    const palette = extractThemeInfo(name, theme).terminalPalette;
    for (const [slot, color] of Object.entries({
      background: palette.background,
      foreground: palette.foreground,
      cursor: palette.cursor,
      cursorText: palette.cursorText,
      ...palette.ansi,
    })) {
      assert.match(color, HEX, `${name} has invalid ${slot}: ${color}`);
    }
    assert.deepEqual(Object.keys(palette.ansi), [...ANSI_COLOR_NAMES]);
  }
});

test("exact ANSI colors win and missing bright colors derive toward contrast", () => {
  const info = extractThemeInfo("fixture", {
    name: "fixture",
    type: "dark",
    colors: {
      "editor.background": "#101010",
      "editor.foreground": "#eeeeee",
      "terminal.ansiRed": "#123456",
    },
    tokenColors: [
      { scope: ["string.quoted"], settings: { foreground: "#00aa33" } },
    ],
  });

  assert.equal(info.terminalPalette.ansi.red, "#123456");
  assert.equal(info.terminalPalette.ansi.green, "#00aa33");
  assert.notEqual(
    info.terminalPalette.ansi.brightRed,
    info.terminalPalette.ansi.red,
  );
});
