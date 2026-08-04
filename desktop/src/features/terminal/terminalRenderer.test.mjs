import assert from "node:assert/strict";
import test from "node:test";

import { TerminalGrid } from "./terminalRenderer.ts";

const palette = {
  background: "#000000",
  foreground: "#ffffff",
  cursor: "#ffffff",
  cursorText: "#000000",
  ansi: Object.fromEntries(
    [
      "black",
      "red",
      "green",
      "yellow",
      "blue",
      "magenta",
      "cyan",
      "white",
      "brightBlack",
      "brightRed",
      "brightGreen",
      "brightYellow",
      "brightBlue",
      "brightMagenta",
      "brightCyan",
      "brightWhite",
    ].map((name, index) => [name, `#${index.toString(16).padStart(6, "0")}`]),
  ),
};

function context() {
  const text = [];
  const fills = [];
  return {
    text,
    fills,
    fillStyle: "",
    font: "",
    textBaseline: "alphabetic",
    save() {},
    restore() {},
    fillRect(...args) {
      fills.push(args);
    },
    fillText(...args) {
      text.push(args);
    },
  };
}

const metrics = {
  width: 10,
  height: 20,
  baseline: 15,
  font: "normal",
  boldFont: "bold",
};

test("clusters draw at computed columns, never accumulated text width", () => {
  const grid = new TerminalGrid({ generation: 0, columns: 12, screenLines: 1 });
  assert.equal(
    grid.apply({
      viewport: grid.viewport,
      full: true,
      cursor: { line: 0, column: 0, visible: false },
      rows: [
        {
          line: 0,
          spans: [
            {
              style: { fg: 0x01000007, bg: 0x01000101, flags: 0 },
              clusters: [
                { column: 0, text: "a", width: 1 },
                { column: 1, text: "😀", width: 2 },
                { column: 3, text: "b", width: 1 },
                { column: 4, text: "一", width: 2 },
                { column: 6, text: "c", width: 1 },
              ],
            },
          ],
        },
      ],
    }),
    true,
  );
  const ctx = context();
  grid.paint(ctx, metrics, palette);
  assert.deepEqual(
    ctx.text.map((call) => [call[0], call[1]]),
    [
      ["a", 0],
      ["😀", 10],
      ["b", 30],
      ["一", 40],
      ["c", 60],
    ],
  );
});

test("combining marks stay in one cluster and consume one cell", () => {
  const grid = new TerminalGrid({ generation: 2, columns: 4, screenLines: 1 });
  grid.apply({
    viewport: grid.viewport,
    full: false,
    cursor: { line: 0, column: 0, visible: false },
    rows: [
      {
        line: 0,
        spans: [
          {
            style: { fg: 0x01000007, bg: 0x01000101, flags: 0 },
            clusters: [{ column: 2, text: "é", width: 1 }],
          },
        ],
      },
    ],
  });
  const ctx = context();
  grid.paint(ctx, metrics, palette);
  assert.deepEqual(ctx.text[0], ["é", 20, 15]);
});

test("stale viewport frames are rejected without mutating retained rows", () => {
  const grid = new TerminalGrid({ generation: 4, columns: 4, screenLines: 1 });
  assert.equal(
    grid.apply({
      viewport: { generation: 3, columns: 4, screenLines: 1 },
      full: true,
      cursor: { line: 0, column: 0, visible: false },
      rows: [],
    }),
    false,
  );
});

test("cursor visibility can blink without a new terminal frame", () => {
  const grid = new TerminalGrid({ generation: 0, columns: 4, screenLines: 1 });
  grid.apply({
    viewport: grid.viewport,
    full: true,
    cursor: { line: 0, column: 2, visible: true },
    rows: [],
  });

  const visible = context();
  grid.paint(visible, metrics, palette);
  assert.ok(visible.fills.some((fill) => fill[0] === 20 && fill[2] === 1.2));

  grid.setCursorPainted(false);
  const hidden = context();
  grid.paint(hidden, metrics, palette);
  assert.equal(
    hidden.fills.some((fill) => fill[0] === 20 && fill[2] === 1.2),
    false,
  );
  assert.ok(
    hidden.fills.some((fill) => fill[0] === 0 && fill[2] === 40),
    "hiding the cursor must repaint its row to erase the old caret",
  );

  grid.setCursorPainted(true);
  const restored = context();
  grid.paint(restored, metrics, palette);
  assert.ok(restored.fills.some((fill) => fill[0] === 20 && fill[2] === 1.2));
});
