import { test } from 'node:test';
import assert from 'node:assert/strict';
import { once } from 'node:events';
import { mkdtempSync, readFileSync, rmSync, statSync, readdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { WebSocket } from 'ws';
import { relay } from '../src/relay.ts';
import { writePrivate } from '../src/storage.ts';
import { message, newKey, publicKey, seal, type Envelope } from '../src/protocol.ts';
import type { SnapshotBoundary } from '../src/relay-storage.ts';

function latch() {
  let release!: () => void;
  const promise = new Promise<void>(resolve => { release = resolve; });
  return { promise, release };
}
async function fixture(boundary?: SnapshotBoundary) {
  const dir = mkdtempSync(join(tmpdir(), 'beehive-relay-async-'));
  const path = join(dir, 'private', 'history.json');
  const secret = newKey();
  const seed = seal(message('inspect', 'h', 'a'), secret);
  writePrivate(path, [seed]); // Existing synchronous format must reopen unchanged.
  const server = await relay(0, publicKey(secret), path, boundary);
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const port = address.port;
  const clients: WebSocket[] = [];
  async function connect() {
    const ws = new WebSocket(`ws://127.0.0.1:${port}`);
    clients.push(ws);
    const seen: Envelope[] = [];
    ws.on('message', data => seen.push(JSON.parse(data.toString())));
    await once(ws, 'open');
    await fence(ws);
    return { ws, seen };
  }
  return { dir, path, secret, seed, server, connect, clients,
    envelope: () => seal(message('inspect', 'h', 'a'), secret),
    disk: (): Envelope[] => JSON.parse(readFileSync(path, 'utf8')),
    async cleanup() {
      for (const ws of clients) ws.terminate();
      for (const ws of server.clients) ws.terminate();
      await new Promise<void>(resolve => server.close(() => resolve()));
      rmSync(dir, { recursive: true, force: true });
    } };
}
async function fence(ws: WebSocket) {
  const pong = once(ws, 'pong'); ws.ping(); await pong;
}
async function received(ws: WebSocket, seen: Envelope[], count: number) {
  while (seen.length < count) await once(ws, 'message');
}

test('pending durable snapshot permits another HTTP upgrade and loop turn; ordered commit, duplicate, reconnect, restart', { timeout: 10000 }, async () => {
  const entered = latch(); const release = latch();
  let writes = 0;
  const f = await fixture(async phase => {
    if (phase === 'write' && ++writes === 1) { entered.release(); await release.promise; }
  });
  try {
    const a = await f.connect(); const first = f.envelope(); const second = f.envelope(); const third = f.envelope();
    a.ws.send(JSON.stringify(first)); await entered.promise;
    // This upgrade and ping/pong MUST complete while the real writer is suspended.
    const b = await f.connect();
    await new Promise<void>(resolve => setImmediate(resolve));
    b.ws.send(JSON.stringify(second)); b.ws.send(JSON.stringify(third)); a.ws.send(JSON.stringify(first));
    await fence(a.ws); await fence(b.ws);
    assert.deepEqual(f.disk(), [f.seed]);
    assert.deepEqual(a.seen, [f.seed]); assert.deepEqual(b.seen, [f.seed]);
    release.release(); await received(b.ws, b.seen, 4);
    assert.deepEqual(b.seen, [f.seed, first, second, third]);
    assert.deepEqual(f.disk(), b.seen); assert.equal(writes, 2);
    const c = await f.connect(); assert.deepEqual(c.seen, b.seen);
    for (const ws of f.clients) ws.terminate();
    await new Promise<void>(resolve => f.server.close(() => resolve()));
    const restarted = await relay(0, publicKey(f.secret), f.path);
    const address = restarted.address(); assert.ok(address && typeof address !== 'string');
    const ws = new WebSocket(`ws://127.0.0.1:${address.port}`); const seen: Envelope[] = [];
    ws.on('message', data => seen.push(JSON.parse(data.toString())));
    await once(ws, 'open'); await fence(ws);
    assert.deepEqual(seen, [f.seed, first, second, third]);
    ws.terminate(); for (const client of restarted.clients) client.terminate();
    await new Promise<void>(resolve => restarted.close(() => resolve()));
    assert.equal(statSync(f.path).mode & 0o777, 0o600);
    assert.equal(statSync(join(f.dir, 'private')).mode & 0o777, 0o700);
    assert.deepEqual(readdirSync(join(f.dir, 'private')), ['history.json']);
  } finally { release.release(); await f.cleanup(); }
});

for (const fault of ['write', 'rename'] as const) test(`storage ${fault} failure has no ghost acceptance and does not poison queued sibling`, { timeout: 10000 }, async () => {
  const entered = latch(); const release = latch(); let failed = false;
  const f = await fixture(async phase => {
    if (phase === fault && !failed) {
      failed = true; entered.release(); await release.promise; throw Error('fixture disk failure');
    }
  });
  try {
    const a = await f.connect(); const b = await f.connect();
    const bad = f.envelope(); const good = f.envelope();
    a.ws.send(JSON.stringify(bad)); await entered.promise;
    b.ws.send(JSON.stringify(good)); await fence(b.ws);
    const closed = once(a.ws, 'close'); release.release();
    assert.equal((await closed)[0], 1011);
    await received(b.ws, b.seen, 2);
    assert.deepEqual(b.seen, [f.seed, good]); assert.deepEqual(f.disk(), b.seen);
    assert.deepEqual(readdirSync(join(f.dir, 'private')), ['history.json']);
  } finally { release.release(); await f.cleanup(); }
});

test('close drains pending file and directory durability; no publication visible before directory fsync', { timeout: 10000 }, async () => {
  const entered = latch(); const release = latch();
  const f = await fixture(async phase => { if (phase === 'directory') { entered.release(); await release.promise; } });
  try {
    const a = await f.connect(); const e = f.envelope(); a.ws.send(JSON.stringify(e)); await entered.promise;
    assert.deepEqual(a.seen, [f.seed]);
    let closed = false;
    const drain = new Promise<void>((resolve, reject) => f.server.close(error => {
      closed = true; if (error) reject(error); else resolve();
    }));
    a.ws.terminate(); await new Promise<void>(resolve => setImmediate(resolve));
    assert.equal(closed, false);
    release.release(); await drain;
    assert.deepEqual(f.disk(), [f.seed, e]);
  } finally { release.release(); await f.cleanup(); }
});

test('post-replacement failure is uncertain, never echoed; siblings fenced and close reports error', { timeout: 10000 }, async () => {
  const entered = latch(); const release = latch();
  const f = await fixture(async phase => {
    if (phase === 'directory') { entered.release(); await release.promise; throw Error('fixture fsync failure'); }
  });
  try {
    const a = await f.connect(); const b = await f.connect();
    const uncertain = f.envelope(); a.ws.send(JSON.stringify(uncertain)); await entered.promise;
    b.ws.send(JSON.stringify(f.envelope())); await fence(b.ws);
    const closed = once(b.ws, 'close'); release.release(); assert.equal((await closed)[0], 1011);
    assert.deepEqual(a.seen, [f.seed]); assert.deepEqual(b.seen, [f.seed]);
    assert.deepEqual(f.disk(), [f.seed, uncertain], 'replacement is possible, not claimed rejected');
    const error = await new Promise<Error | undefined>(resolve => f.server.close(resolve));
    assert.match(String(error), /durability uncertain/);
  } finally { release.release(); await f.cleanup(); }
});
