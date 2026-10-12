import assert from "node:assert/strict";
import test from "node:test";
import { stats, termProbe } from "./termProbe.ts";
import { kbDebugFromUrl } from "./kbDebug.ts";

test("stats: avg / p95 / last", () => {
  const s = stats([10, 20, 30, 40, 100]);
  assert.equal(s.n, 5); assert.equal(s.avg, 40); assert.equal(s.p95, 100); assert.equal(s.last, 100);
  assert.equal(stats([]).n, 0);
});
test("probe: a held key is timed from the first unanswered key to echo and paint; off = no-op", async () => {
  termProbe.reset(); termProbe.setEnabled(false);
  termProbe.inputSent(); termProbe.frameArrived(); termProbe.painted();
  assert.equal(termProbe.snapshot().echo.n, 0);
  termProbe.setEnabled(true);
  termProbe.inputSent(); await new Promise((r) => setTimeout(r, 20)); termProbe.inputSent();
  termProbe.frameArrived(); termProbe.painted();
  const s = termProbe.snapshot();
  assert.equal(s.echo.n, 1); assert.equal(s.paint.n, 1);
  assert.ok(s.echo.last >= 18, String(s.echo.last));
  termProbe.painted(); // no pending input: ignored
  assert.equal(termProbe.snapshot().paint.n, 1);
  termProbe.setEnabled(false);
});
test("kbdebug URL switch in query or hash route", () => {
  assert.equal(kbDebugFromUrl("https://x/?kbdebug=1"), true);
  assert.equal(kbDebugFromUrl("https://x/#/channels/a?kbdebug=0"), false);
  assert.equal(kbDebugFromUrl("https://x/#/channels/a"), null);
});
