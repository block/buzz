import { provisionSetup } from './provision.ts';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, realpathSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { host } from '../src/host.ts';
import { connect } from '../src/client.ts';
import { relay } from '../src/relay.ts';
import { message, newKey, publicKey, type Message } from '../src/protocol.ts';
import { writePrivate } from '../src/storage.ts';

async function until(predicate: () => boolean) {
  for (let i = 0; i < 200; i++) { if (predicate()) return; await delay(25); }
  throw Error('Missing reconnect convergence evidence');
}

test('host reconnect replays durable relay intent and receipts without repeating lifecycle effects', async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-reconnect-')));
  const secret = newKey(); const agentSecret = newKey(); const agent = publicKey(agentSecret);
  provisionSetup(join(dir, 'setup.json'), { host: 'reconnect-host', ownerSecret: secret, agentSecret,
    runner: process.execPath, args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' });
  const server = await relay(0, publicKey(secret), join(dir, 'relay.json'));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`;
  const h = await host(dir, url);
  const hostSocket = [...server.clients][0]; assert.ok(hostSocket);
  const seen: Message[] = [];
  const ui = connect(url, secret, m => seen.push(m)); await ui.ready;
  const uiSocket = [...server.clients].find(s => s !== hostSocket); assert.ok(uiSocket);
  const journal = () => JSON.parse(readFileSync(join(dir, 'journal.json'), 'utf8'));
  try {
    // The relay accepts intent while the host is disconnected. No caller retry
    // or new operation ID is required to get the saved operation applied.
    hostSocket.terminate();
    const start = message('start', 'reconnect-host', agent, 0);
    ui.send(start);
    await until(() => seen.some(m => m.type === 'receipt' && m.body.operation === start.id));
    assert.equal(journal().revision, 1); assert.equal(journal().phase, 'running');
    const actual = journal().actual;
    const receipt = journal().operations[start.id].reply;
    assert.equal(receipt.body.result, 'accepted');
    // Lose the host link again after commit. Replay must reuse the committed
    // receipt and actual run, even with old Start in the relay's full history.
    seen.length = 0;
    const nextHostSocket = [...server.clients].find(s => s !== hostSocket && s !== uiSocket);
    assert.ok(nextHostSocket); nextHostSocket.terminate();
    await until(() => seen.some(m => m.id === receipt.id));
    assert.equal(journal().revision, 1); assert.deepEqual(journal().actual, actual);
    assert.deepEqual(Object.keys(journal().operations), [start.id]);
    const stop = message('stop', 'reconnect-host', agent, 1); ui.send(stop);
    await until(() => seen.some(m => m.type === 'receipt' && m.body.operation === stop.id));
    assert.equal(journal().phase, 'stopped'); assert.equal(journal().revision, 2);
    await h.close();
    await delay(300);
    assert.equal(server.clients.size, 1, 'intentional host close must not reconnect');
  } finally {
    await h.close(); ui.close(); for (const socket of server.clients) socket.terminate();
    await new Promise<void>(resolve => server.close(() => resolve()));
    rmSync(dir, { recursive: true, force: true });
  }
});

test('intentional client close cancels a scheduled reconnect', async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-reconnect-close-')));
  const secret = newKey(); const server = await relay(0, publicKey(secret), join(dir, 'relay.json'));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  let recovered = 0;
  const client = connect(`ws://127.0.0.1:${address.port}`, secret, () => {}, () => { recovered++; });
  try {
    await client.ready;
    const disconnected = new Promise<void>(resolve => client.socket.once('close', () => resolve()));
    for (const socket of server.clients) socket.terminate();
    await disconnected; client.close(); await delay(300);
    assert.equal(recovered, 0); assert.equal(server.clients.size, 0);
  } finally {
    client.close(); for (const socket of server.clients) socket.terminate();
    await new Promise<void>(resolve => server.close(() => resolve()));
    rmSync(dir, { recursive: true, force: true });
  }
});
