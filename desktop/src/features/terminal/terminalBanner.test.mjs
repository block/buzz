import assert from "node:assert/strict";
import test from "node:test";

import { buildTerminalBanner } from "./terminalBanner.ts";

function layers(banner) {
  return new Map(
    ["field", "head", "bevel_hi", "bevel_lo"].map((layer) => [
      layer,
      banner.cells.flat().filter((cell) => cell.layer === layer),
    ]),
  );
}

function frameBounds(banner) {
  const cells = banner.cells.flatMap((row, y) =>
    row.flatMap((cell, x) =>
      cell.layer === "bevel_hi" || cell.layer === "bevel_lo" ? [{ x, y }] : [],
    ),
  );
  return {
    top: Math.min(...cells.map(({ y }) => y)),
    left: Math.min(...cells.map(({ x }) => x)),
    bottom: Math.max(...cells.map(({ y }) => y)) + 1,
    right: Math.max(...cells.map(({ x }) => x)) + 1,
  };
}

function headRows(banner) {
  const cells = banner.cells.flatMap((row, y) =>
    row.flatMap((cell, x) =>
      cell.layer === "head" ? [{ ...cell, x, y }] : [],
    ),
  );
  const left = Math.min(...cells.map(({ x }) => x));
  const right = Math.max(...cells.map(({ x }) => x));
  const top = Math.min(...cells.map(({ y }) => y));
  const bottom = Math.max(...cells.map(({ y }) => y));
  return Array.from({ length: bottom - top + 1 }, (_, row) =>
    Array.from(
      { length: right - left + 1 },
      (_, column) =>
        cells.find(({ x, y }) => x === left + column && y === top + row)
          ?.char ?? " ",
    )
      .join("")
      .trimEnd(),
  );
}

const EXPECTED_BUZZ_TERM = [
  "██                                             ██",
  "██▄▄▄     ██  ██    ██████    ██████          █████      ▄███▄    ██ ▄██    ██▄██▄██",
  "██▀▀██    ██  ██       ▄██       ▄██           ██       ██▄▄▄█    ███▀▀     ██ ██ ██",
  "██  ██    ██  ██     ▄██▀      ▄██▀            ██       ██        ██        ██ ██ ██",
  "██████    ▀█████    ██████    ██████            ███      ▀███▀    ██        ██ ██ ██",
];

test("builds the amended four-layer Buzz Term composite", () => {
  const banner = buildTerminalBanner(183, 69, 17 / 8.4);
  assert.ok(banner);
  const seen = layers(banner);
  for (const layer of seen.keys()) assert.ok(seen.get(layer).length > 0, layer);
  const head = seen.get("head");
  const sweep = head.map((cell) => cell.t);
  assert.equal(Math.min(...sweep), 0);
  assert.equal(Math.max(...sweep), 1);
  assert.ok(head.every((cell) => cell.layer === "head"));
  const { top, left, bottom, right } = frameBounds(banner);
  assert.equal(banner.cells[top][left].layer, "bevel_hi");
  assert.equal(banner.cells[bottom - 1][right - 1].layer, "bevel_lo");
  assert.deepEqual(headRows(banner), EXPECTED_BUZZ_TERM);
});

test("requires every frame emitter independently", () => {
  const banner = buildTerminalBanner(183, 69, 17 / 8.4);
  assert.ok(banner);
  const { top, left, bottom, right } = frameBounds(banner);

  for (let x = left; x < right; x += 1)
    assert.equal(banner.cells[top][x].layer, "bevel_hi", `top_row:${x}`);
  for (let y = top + 1; y < bottom - 1; y += 1) {
    assert.equal(banner.cells[y][left].layer, "bevel_hi", `left_rail:${y}`);
    assert.equal(
      banner.cells[y][right - 1].layer,
      "bevel_lo",
      `right_rail:${y}`,
    );
  }
  for (let x = left; x < right; x += 1)
    assert.equal(
      banner.cells[bottom - 1][x].layer,
      "bevel_lo",
      `bottom_row:${x}`,
    );
});

test("emits complete hexagons only and fades per ray", () => {
  const banner = buildTerminalBanner(112, 46, 17 / 8.4);
  assert.ok(banner);
  const field = layers(banner).get("field");
  assert.equal(field.length % 8, 0);
  assert.ok(field.some((cell) => cell.t < 0.1));
  assert.ok(field.some((cell) => cell.t > 0.5));
});

test("declines viewports too small for the complete wordmark", () => {
  assert.equal(buildTerminalBanner(40, 20, 17 / 8.4), null);
});
