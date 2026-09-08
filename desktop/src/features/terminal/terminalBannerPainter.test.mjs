import assert from "node:assert/strict";
import test from "node:test";

import { buildTerminalBanner } from "./terminalBanner.ts";
import { paintTerminalBanner } from "./terminalBannerPainter.ts";

const palette = {
  background: "#10141c",
  foreground: "#d0d0d0",
  cursor: "#ffffff",
  cursorText: "#000000",
  ansi: {
    black: "#10141c",
    red: "#f07178",
    green: "#aad94c",
    yellow: "#ffb454",
    blue: "#59c2ff",
    magenta: "#d2a6ff",
    cyan: "#95e6cb",
    white: "#d0d0d0",
    brightBlack: "#4d5566",
    brightRed: "#f07178",
    brightGreen: "#aad94c",
    brightYellow: "#ffb454",
    brightBlue: "#59c2ff",
    brightMagenta: "#d2a6ff",
    brightCyan: "#95e6cb",
    brightWhite: "#ffffff",
  },
};

function canvas() {
  const draws = [];
  const clears = [];
  const colors = [];
  const fills = [];
  const context = {
    _fillStyle: "",
    clearRect(...args) {
      clears.push(args);
    },
    fillRect(...args) {
      fills.push({ args, color: this._fillStyle, glyphsBefore: draws.length });
    },
    fillText(...args) {
      draws.push(args);
      colors.push(this._fillStyle);
    },
    font: "",
    set fillStyle(value) {
      this._fillStyle = value;
    },
    get fillStyle() {
      return this._fillStyle;
    },
    setTransform() {},
    textBaseline: "",
  };
  return {
    canvas: {
      height: 0,
      width: 0,
      getBoundingClientRect: () => ({ height: 600, width: 1000 }),
      getContext: () => context,
    },
    clears,
    colors,
    draws,
    fills,
  };
}

test("paints every emitted banner glyph at terminal cell coordinates", () => {
  const banner = buildTerminalBanner(112, 46, 17 / 8.4);
  assert.ok(banner);
  const emitted = banner.cells.flat().filter((cell) => cell.char !== " ");
  const target = canvas();
  assert.equal(paintTerminalBanner(target.canvas, banner, palette, 2), true);
  assert.equal(target.draws.length, emitted.length);
  assert.ok(target.draws.some(([char]) => char === "█"));
  assert.ok(target.draws.some(([char]) => char === "/"));
  assert.ok(target.draws.some(([, x, y]) => x > 0 && y > 0));
  assert.equal(target.canvas.width, 2000);
  assert.equal(target.canvas.height, 1200);
  assert.deepEqual(target.clears, [[0, 0, 1000, 600]]);
});

test("lays an opaque theme background under the banner before any glyph", () => {
  const banner = buildTerminalBanner(112, 46, 17 / 8.4);
  assert.ok(banner);
  const target = canvas();
  assert.equal(paintTerminalBanner(target.canvas, banner, palette, 2), true);
  assert.equal(target.fills.length, 1);
  const [ground] = target.fills;
  assert.deepEqual(ground.args, [0, 0, 1000, 600]);
  assert.equal(ground.color, palette.background);
  assert.equal(ground.glyphsBefore, 0);
  assert.ok(target.draws.length > 0);
});

test("uses visibly distinct theme-derived colors across banner layers", () => {
  const banner = buildTerminalBanner(112, 46, 17 / 8.4);
  assert.ok(banner);
  const target = canvas();
  paintTerminalBanner(target.canvas, banner, palette, 1);
  assert.ok(new Set(target.colors).size >= 3);
  assert.equal(target.colors.includes(""), false);
});

test("fails closed when the banner canvas has no 2d context", () => {
  const banner = buildTerminalBanner(112, 46, 17 / 8.4);
  assert.ok(banner);
  assert.equal(
    paintTerminalBanner(
      {
        height: 0,
        width: 0,
        getBoundingClientRect: () => ({ height: 600, width: 1000 }),
        getContext: () => null,
      },
      banner,
      palette,
      1,
    ),
    false,
  );
});
