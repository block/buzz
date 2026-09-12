import { test } from "node:test";
import assert from "node:assert/strict";
import { CaptureQueue } from "./capture-queue.mjs";
test("1500ms scheduler stall retains ordered samples and drains in bounded ACP frames", () => {
  const q = new CaptureQueue();
  for (let i = 0; i < 75; i++) q.push(new Int16Array(480).fill(i));
  const values = [];
  while (q.length) {
    const pcm = q.shift();
    assert.ok(pcm.length <= 2400);
    values.push(...pcm);
  }
  assert.equal(values.length, 75 * 480);
  for (let i = 0; i < values.length; i++)
    assert.equal(values[i], Math.floor(i / 480));
});
test("five seconds is a hard bound, overflow is visible and no accepted audio is lost", () => {
  const q = new CaptureQueue();
  for (let i = 0; i < 250; i++) q.push(new Int16Array(480));
  assert.throws(() => q.push(new Int16Array(480)), /stalled/);
  assert.equal(q.length, 120000);
  let n = 0;
  while (q.length) n += q.shift().length;
  assert.equal(n, 120000);
});
