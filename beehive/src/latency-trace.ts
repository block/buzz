import { writeFileSync, openSync, fsyncSync, closeSync } from 'node:fs';
import { join } from 'node:path';
let active = false;
let name = '';
let rows: unknown[] = [];
let timer: ReturnType<typeof setInterval> | undefined;
let dropped = 0;
/** Bounded opt-in event timing; callers must never pass payloads or credentials. */
export function trace(phase: string, data: unknown = null) {
  if (!active) return;
  if (rows.length >= 50000) { dropped++; return; }
  rows.push({ phase, wall: Date.now(), mono: performance.now(), data });
}
/** Tests explicitly activate capture in an operator-provided private fixture directory. */
export function beginTrace(label: string) {
  if (!process.env.BEEHIVE_LATENCY_TRACE) return;
  active = true; name = label; rows = []; dropped = 0;
  trace('trace.begin', { pid: process.pid, label });
  let expected = performance.now() + 25;
  timer = setInterval(() => { trace('loop.tick', { late: performance.now() - expected }); expected = performance.now() + 25; }, 25);
  timer.unref();
}
/** Exclusive durable artifact, including on test failure; never discard an earlier capture. */
export function endTrace() {
  if (!active) return;
  clearInterval(timer); trace('trace.end', { dropped }); active = false;
  const directory = process.env.BEEHIVE_LATENCY_TRACE!;
  const fd = openSync(join(directory, `${process.pid}-${name}.json`), 'wx', 0o600);
  try { writeFileSync(fd, JSON.stringify(rows)); fsyncSync(fd); } finally { closeSync(fd); }
  const parent = openSync(directory, 'r');
  try { fsyncSync(parent); } finally { closeSync(parent); }
}
