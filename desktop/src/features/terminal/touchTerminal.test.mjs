import assert from "node:assert/strict";
import test from "node:test";
import { TouchGestures, applyStickyModifiers, fontSizeForPinch, keyboardInset, TOUCH_KEYS } from "./touchTerminal.ts";

test("tap: a short touch with no travel is a tap at the touch point", () => {
  const g = new TouchGestures();
  g.down(1, 10, 20, 0);
  assert.deepEqual(g.up(1, 120), [{ type: "tap", x: 10, y: 20 }]);
  assert.equal(g.state, "idle");
});

test("drag past the slop scrolls by the finger travel, incrementally, and never taps", () => {
  const g = new TouchGestures({ slopPx: 8 });
  g.down(1, 10, 100, 0);
  assert.deepEqual(g.move(1, 12, 104, 10), []);
  assert.deepEqual(g.move(1, 12, 80, 20), [{ type: "scroll", dy: 20 }]);
  assert.deepEqual(g.move(1, 12, 70, 30), [{ type: "scroll", dy: 10 }]);
  assert.deepEqual(g.up(1, 40), []);
});

test("long press enters selection; moving extends; release ends it; a press that moved first never selects", () => {
  const g = new TouchGestures({ longPressMs: 450 });
  g.down(1, 50, 60, 0);
  assert.deepEqual(g.timeout(100), []);
  assert.deepEqual(g.timeout(450), [{ type: "longpress", x: 50, y: 60 }]);
  assert.deepEqual(g.move(1, 90, 60, 500), [{ type: "select", x: 90, y: 60 }]);
  assert.deepEqual(g.up(1, 600), [{ type: "selectEnd", x: 90, y: 60 }]);
  g.down(2, 5, 5, 1000);
  g.move(2, 5, 50, 1100);
  assert.deepEqual(g.timeout(1500), []);
});

test("a slow tap (held past the tap window without moving or timing out) isn't a tap", () => {
  const g = new TouchGestures({ tapMaxMs: 400 });
  g.down(1, 1, 1, 0);
  assert.deepEqual(g.up(1, 900), []);
});

test("pinch reports the cumulative scale; lifting one finger ends it and the other can't scroll or tap", () => {
  const g = new TouchGestures();
  g.down(1, 100, 100, 0);
  g.down(2, 200, 100, 5);
  assert.deepEqual(g.move(2, 300, 100, 10), [{ type: "pinch", scale: 2 }]);
  assert.deepEqual(g.move(1, 150, 100, 20), [{ type: "pinch", scale: 1.5 }]);
  assert.deepEqual(g.up(2, 30), [{ type: "pinchEnd" }]);
  assert.deepEqual(g.move(1, 150, 10, 40), []);
  assert.deepEqual(g.up(1, 50), []);
  assert.equal(g.state, "idle");
});

test("a second finger during selection cancels the selection", () => {
  const g = new TouchGestures();
  g.down(1, 5, 5, 0);
  g.timeout(500);
  assert.deepEqual(g.down(2, 50, 5, 520), [{ type: "cancel" }]);
});

test("sticky Ctrl turns the next letter into its control code; Alt prefixes ESC; unmappable keys keep Ctrl armed", () => {
  assert.deepEqual(applyStickyModifiers("c", { ctrl: true, alt: false }), { text: "\u0003", used: true });
  assert.deepEqual(applyStickyModifiers("C", { ctrl: true, alt: false }), { text: "\u0003", used: true });
  assert.deepEqual(applyStickyModifiers("[", { ctrl: true, alt: false }), { text: "\u001b", used: true });
  assert.deepEqual(applyStickyModifiers(" ", { ctrl: true, alt: false }), { text: "\u0000", used: true });
  assert.deepEqual(applyStickyModifiers("1", { ctrl: true, alt: false }), { text: "1", used: false });
  assert.deepEqual(applyStickyModifiers("hello", { ctrl: true, alt: false }), { text: "hello", used: false });
  assert.deepEqual(applyStickyModifiers("f", { ctrl: false, alt: true }), { text: "\u001bf", used: true });
  assert.deepEqual(applyStickyModifiers("b", { ctrl: true, alt: true }), { text: "\u001b\u0002", used: true });
  assert.deepEqual(applyStickyModifiers("x", { ctrl: false, alt: false }), { text: "x", used: false });
  assert.equal(TOUCH_KEYS.up, "\u001b[A");
});

test("pinch font size scales, rounds and clamps", () => {
  assert.equal(fontSizeForPinch(14, 1.5), 21);
  assert.equal(fontSizeForPinch(14, 0.1), 8);
  assert.equal(fontSizeForPinch(14, 9), 28);
  assert.equal(fontSizeForPinch(14, Number.NaN), 14);
});

test("keyboard inset: the on-screen keyboard shows up as a much shorter visual viewport", () => {
  assert.equal(keyboardInset({ height: 800, offsetTop: 0, width: 390 }, 800).open, false);
  const k = keyboardInset({ height: 470, offsetTop: 0, width: 390 }, 800);
  assert.deepEqual([k.open, k.height], [true, 470]);
  assert.equal(keyboardInset(null, 800).open, false);
});
