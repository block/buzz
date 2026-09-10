import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { type Duplex } from 'node:stream';
import { once } from 'node:events';
import { mkdtempSync, realpathSync, readFileSync, existsSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { WebSocketServer } from 'ws';
import { connect } from '../src/client.ts';
import { host, provision } from '../src/host.ts';
import { createGenesis } from '../src/assignment.ts';
import { newKey, publicKey } from '../src/protocol.ts';

// Own only fresh loopback sockets. Withhold HTTP upgrade deliberately, rather
// than blocking the test/relay event loop or shortening production's deadline.
async function endpoint() {
  const sockets = new Set<Duplex>();
  const server = createServer();
  const ws = new WebSocketServer({ noServer: true });
  let mode: 'stall' | 'accept' | 'deny' = 'stall';
  let upgrades = 0;
  server.on('connection', socket => { sockets.add(socket); socket.on('close', () => sockets.delete(socket)); });
  server.on('upgrade', (request, socket, head) => {
    upgrades++;
    if (mode === 'accept') ws.handleUpgrade(request, socket, head, client => ws.emit('connection', client, request));
    else if (mode === 'deny') socket.end('HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n');
  });
  server.listen(0, '127.0.0.1'); await once(server, 'listening');
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  return {
    url: `ws://127.0.0.1:${address.port}`,
    get upgrades() { return upgrades; },
    set mode(value: 'stall' | 'accept' | 'deny') { mode = value; },
    async close() {
      for (const client of ws.clients) client.terminate();
      for (const socket of sockets) socket.destroy();
      await new Promise<void>(resolve => ws.close(() => resolve()));
      await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
    },
  };
}

async function until(predicate: () => boolean) {
  for (let i = 0; i < 200; i++) { if (predicate()) return; await delay(10); }
  throw Error('Missing controlled HTTP upgrade');
}

test('initial handshake timeout is terminal: installation lock unwinds, journal unchanged, explicit new host succeeds', { timeout: 10000 }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-startup-')));
  const secret = newKey(), agentSecret = newKey();
  const e = await endpoint();
  let running: Awaited<ReturnType<typeof host>> | undefined;
  try {
    provision(dir, { host: 'startup', ownerSecret: secret, agentSecret, runner: process.execPath,
      args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' },
    createGenesis(publicKey(secret), publicKey(agentSecret), 'startup'));
    const before = readFileSync(join(dir, 'journal.json'));
    const pending = host(dir, e.url);
    const rejected = assert.rejects(pending, /Opening handshake has timed out/);
    await until(() => e.upgrades === 1);
    assert.equal(existsSync(join(dir, 'host.lock')), true);
    await assert.rejects(host(dir, e.url), /EEXIST/);
    await rejected;
    assert.equal(existsSync(join(dir, 'host.lock')), false);
    assert.deepEqual(readFileSync(join(dir, 'journal.json')), before);
    await delay(350); assert.equal(e.upgrades, 1, 'initial failure must not silently enroll in established recovery');
    e.mode = 'accept'; running = await host(dir, e.url);
    assert.equal(e.upgrades, 2);
    assert.equal(existsSync(join(dir, 'host.lock')), true);
    assert.deepEqual(readFileSync(join(dir, 'journal.json')), before);
    await running.close(); running = undefined;
    assert.equal(existsSync(join(dir, 'host.lock')), false);
  } finally {
    await running?.close(); await e.close(); rmSync(dir, { recursive: true, force: true });
  }
});

test('closing an outstanding initial handshake rejects ready and never recovers', { timeout: 5000 }, async () => {
  const e = await endpoint(); let recovered = 0;
  const client = connect(e.url, newKey(), () => {}, () => { recovered++; });
  const rejected = assert.rejects(client.ready, /closed before the connection was established/);
  try {
    await until(() => e.upgrades === 1);
    client.close(); await rejected;
    e.mode = 'accept'; await delay(350);
    assert.equal(e.upgrades, 1); assert.equal(recovered, 0);
  } finally { client.close(); await e.close(); }
});

test('initial HTTP policy denial fails explicitly without automatic retry even with recovery enabled', { timeout: 5000 }, async () => {
  const e = await endpoint(); e.mode = 'deny'; let recovered = 0;
  const client = connect(e.url, newKey(), () => {}, () => { recovered++; });
  try {
    await assert.rejects(client.ready, /Unexpected server response: 403/);
    e.mode = 'accept'; await delay(350);
    assert.equal(e.upgrades, 1); assert.equal(recovered, 0);
  } finally { client.close(); await e.close(); }
});

for (const signal of ['SIGINT', 'SIGTERM'] as const) test(`actual host CLI ${signal} during withheld upgrade drains startup; subsequent ready shutdown`, { timeout: 10000 }, async () => {
  const { spawn } = await import('node:child_process');
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-cli-startup-')));
  const secret = newKey(), agentSecret = newKey();
  const e = await endpoint();
  provision(dir, { host: 'startup', ownerSecret: secret, agentSecret, runner: process.execPath,
    args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' },
  createGenesis(publicKey(secret), publicKey(agentSecret), 'startup'));
  const before = readFileSync(join(dir, 'journal.json'));
  const launch = () => {
    const child = spawn(process.execPath, ['src/cli.ts', 'host', dir, e.url], { stdio: ['ignore', 'pipe', 'pipe'] });
    let output = '', errors = '';
    child.stdout.on('data', b => output = (output + b).slice(-8192)); child.stderr.on('data', b => errors = (errors + b).slice(-8192));
    return { child, exited: once(child, 'exit'), get output() { return output; }, get errors() { return errors; } };
  };
  let running = launch();
  try {
    await until(() => e.upgrades === 1);
    assert.ok(existsSync(join(dir, 'host.lock')));
    running.child.kill(signal);
    assert.deepEqual(await running.exited, [0, null], running.errors);
    assert.equal(running.errors, '');
    assert.equal(existsSync(join(dir, 'host.lock')), false);
    assert.deepEqual(readFileSync(join(dir, 'journal.json')), before);
    assert.equal(e.upgrades, 1);
    e.mode = 'accept'; running = launch();
    await until(() => running.output.includes('Host online;'));
    running.child.kill(signal);
    assert.deepEqual(await running.exited, [0, null], running.errors);
    assert.equal(existsSync(join(dir, 'host.lock')), false);
    assert.deepEqual(readFileSync(join(dir, 'journal.json')), before);
  } finally {
    if (running.child.exitCode === null && running.child.signalCode === null) { running.child.kill('SIGTERM'); await running.exited; }
    await e.close(); rmSync(dir, { recursive: true, force: true });
  }
});

test('startup abort retains quarantine lock; ready double close shares teardown', async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-abort-fence-')));
  const secret = newKey(), agentSecret = newKey();
  const e = await endpoint();
  const { writePrivate, readPrivate } = await import('../src/storage.ts');
  try {
    provision(dir, { host: 'startup', ownerSecret: secret, agentSecret, runner: process.execPath,
      args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' },
    createGenesis(publicKey(secret), publicKey(agentSecret), 'startup'));
    e.mode = 'accept'; const clean = await host(dir, e.url);
    const one = clean.close(); assert.equal(clean.close(), one); await one;
    const path = join(dir, 'journal.json');
    const state = readPrivate(path) as Record<string, unknown>;
    state.phase = 'quarantined'; writePrivate(path, state);
    e.mode = 'stall'; const controller = new AbortController();
    const pending = host(dir, e.url, controller.signal);
    const rejected = assert.rejects(pending, /Incomplete owned teardown/);
    await until(() => e.upgrades === 2); controller.abort(); await rejected;
    assert.equal(existsSync(join(dir, 'host.lock')), true, 'unproven execution remains fenced');
  } finally { await e.close(); rmSync(dir, { recursive: true, force: true }); }
});
