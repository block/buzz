import assert from "node:assert/strict";
import test from "node:test";
import { createFrameCoalescer, createInputCoalescer, mergeFrames } from "./webTermCoalesce.ts";

const vp = { generation: 1, columns: 80, screenLines: 24 };
const row = (line, t) => ({ line, wrapped: false, spans: [{ style: { fg: 0, bg: 0, flags: 0 }, clusters: [{ column: 0, text: t, width: 1 }] }] });
const fr = (seq, rows, extra = {}) => ({ viewport: vp, rows, cursor: { line: 0, column: seq, visible: true }, full: false, sequence: seq, ...extra });

test("partial frames merge by line, latest wins, cursor from the newest", () => {
  const m = mergeFrames(fr(1, [row(0, "a"), row(3, "x")]), fr(2, [row(0, "b"), row(5, "y")]));
  assert.deepEqual(m.rows.map((r) => [r.line, r.spans[0].clusters[0].text]), [[0, "b"], [3, "x"], [5, "y"]]);
  assert.equal(m.sequence, 2);
  assert.equal(m.cursor.column, 2);
  assert.equal(mergeFrames(fr(1, []), fr(2, [], { full: true })).full, true);
  assert.equal(mergeFrames(fr(1, []), fr(2, [], { viewport: { ...vp, columns: 40 } })), null);
});

test("a burst of 30 echo frames renders once with every row and the last ack", () => {
  const out = []; let run = null;
  const c = createFrameCoalescer((d) => out.push(d), (fn) => { run = fn; });
  const acks = [];
  for (let i = 0; i < 30; i++) c.push({ frame: fr(i, [row(i % 24, String(i))]), acknowledge: async () => acks.push(i) });
  run();
  assert.equal(out.length, 1);
  assert.equal(out[0].frame.sequence, 29);
  assert.equal(out[0].frame.rows.length, 24);
  out[0].acknowledge();
  assert.deepEqual(acks, [29]);
});

test("viewport change flushes the pending frame first (nothing lost)", () => {
  const out = []; const c = createFrameCoalescer((d) => out.push(d.frame.sequence), () => {});
  c.push({ frame: fr(1, [row(0, "a")]), acknowledge: async () => {} });
  c.push({ frame: fr(2, [], { viewport: { ...vp, generation: 2 } }), acknowledge: async () => {} });
  c.flush();
  assert.deepEqual(out, [1, 2]);
});

test("input: first key immediately, a repeat burst becomes few messages, order kept", () => {
  const sent = []; const timers = [];
  const input = createInputCoalescer((t) => sent.push(t), 12, (fn) => timers.push(fn));
  input("\x7f");
  assert.deepEqual(sent, ["\x7f"]); // no added latency for the first key
  for (let i = 0; i < 40; i++) input("\x7f");
  timers.shift()();
  assert.equal(sent.length, 2);
  assert.equal(sent.join(""), "\x7f".repeat(41));
  timers.shift()(); // window closes when idle
  input("a");
  assert.equal(sent.at(-1), "a");
});
