import assert from "node:assert/strict";
import test from "node:test";
import { bottomClearancePx, parseSignerToastPrefs, signerToastDelayMs } from "./signerToastPrefs.ts";

test("prefs default to Only problems / Bottom and survive junk", () => {
  assert.deepEqual(parseSignerToastPrefs(null), { mode: "problems", position: "bottom" });
  assert.deepEqual(parseSignerToastPrefs("{bad"), { mode: "problems", position: "bottom" });
  assert.deepEqual(parseSignerToastPrefs('{"mode":"off","position":"top"}'), { mode: "off", position: "top" });
  assert.deepEqual(parseSignerToastPrefs('{"mode":"loud"}'), { mode: "problems", position: "bottom" });
});
test("show delays: always 1.5s, problems 5s, off never; approval needed always shows now", () => {
  assert.equal(signerToastDelayMs("always", false), 1500);
  assert.equal(signerToastDelayMs("problems", false), 5000);
  assert.equal(signerToastDelayMs("off", false), null);
  for (const m of ["always", "problems", "off"]) assert.equal(signerToastDelayMs(m, true), 0);
});
test("bottom clears the highest docked bar and the safe area", () => {
  assert.equal(bottomClearancePx(844, [770, 700], 34), 152); // above the terminal keys at 700
  assert.equal(bottomClearancePx(844, [], 34), 42);
  assert.equal(bottomClearancePx(844, [100], 34), 42); // top-of-screen elements ignored
});
