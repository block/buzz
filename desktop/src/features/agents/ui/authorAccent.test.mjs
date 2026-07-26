import assert from "node:assert/strict";
import test from "node:test";

import { authorAccent, authorHue } from "./authorAccent.ts";

test("same pubkey always yields the same hue", () => {
  const key = "aa".repeat(32);
  assert.equal(authorHue(key), authorHue(key));
});

test("different pubkeys generally differ", () => {
  const a = authorHue("aa".repeat(32));
  const b = authorHue("bb".repeat(32));
  const c = authorHue("cc".repeat(32));
  assert.equal(new Set([a, b, c]).size >= 2, true);
});

test("hue stays within range", () => {
  for (const key of ["", "a", "ff".repeat(32), "zzz"]) {
    const hue = authorHue(key);
    assert.equal(hue >= 0 && hue < 360, true, `out of range for ${key}`);
    assert.equal(Number.isInteger(hue), true);
  }
});

test("missing pubkey is handled without throwing", () => {
  assert.equal(authorHue(null), 0);
  assert.equal(authorHue(undefined), 0);
});

test("accent returns usable css colour strings", () => {
  const accent = authorAccent("aa".repeat(32));
  assert.match(accent.border, /^hsl\(\d+ 70% 55%\)$/);
  assert.match(accent.background, /^hsl\(\d+ 70% 55% \/ 0\.10\)$/);
});
