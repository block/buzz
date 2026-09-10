import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, realpathSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { host } from '../src/host.ts';
import { managementClient } from '../src/intents.ts';
import { connect } from '../src/client.ts';
import { relay } from '../src/relay.ts';
import { message, newKey, publicKey, open, type Message } from '../src/protocol.ts';
import { writePrivate } from '../src/storage.ts';

async function until(predicate: () => boolean) {
  for (let i = 0; i < 200; i++) { if (predicate()) return; await delay(25); }
  throw Error('Missing reconnect convergence evidence');
}

test('durable client reopens exact unpublished intent, recovers lost post-commit receipt, and fences unrelated results', async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-intents-')));
  const secret = newKey(); const agentSecret = newKey(); const agent = publicKey(agentSecret);
  writePrivate(join(dir, 'setup.json'), { host: 'reconnect-host', ownerSecret: secret, agentSecret,
    runner: process.execPath, args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' });
  const server = await relay(0, publicKey(secret), join(dir, 'relay.json'));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`;
  const h = await host(dir, url);
  let ui = managementClient(join(dir, 'client'), url, secret, () => {}); await ui.ready;
  const journal = () => JSON.parse(readFileSync(join(dir, 'journal.json'), 'utf8'));
  try {
    // Actual dropped WS before publication; synchronous submission persists offline.
    ui.socket.terminate();
    const start = message('start', 'reconnect-host', agent, 0);
    ui.submit(start);
    assert.equal(ui.status()[0]?.state, 'pending');
    ui.close(); // Quit is not Stop and does not delete the unsent operation.
    assert.equal(journal().revision, 0);
    ui = managementClient(join(dir, 'client'), url, secret, () => {}); await ui.ready;
    await until(() => ui.status()[0]?.state === 'completed');
    assert.equal(journal().revision, 1);
    const actual = journal().actual;
    const receipt = journal().operations[start.id].reply;
    const history = () => JSON.parse(readFileSync(join(dir, 'relay.json'), 'utf8'));
    const before = history().length;
    ui.socket.terminate(); ui.close();
    ui = managementClient(join(dir, 'client'), url, secret, () => {}); await ui.ready;
    await delay(150);
    assert.equal(ui.status()[0]?.state, 'completed');
    assert.deepEqual(journal().actual, actual);
    assert.deepEqual(journal().operations[start.id].reply, receipt);
    assert.equal(journal().revision, 1);
    // Permanent host rejection stays terminal, never regenerated on reconnect.
    const rejected = message('start', 'reconnect-host', agent, 0); ui.submit(rejected);
    await until(() => ui.status().find(s => s.request.id === rejected.id)?.state === 'failed');
    const unrelated = connect(url, secret, () => {}); await unrelated.ready;
    unrelated.send(message('receipt', 'other-host', agent, 1, { ...receipt.body, operation: rejected.id }));
    unrelated.send(receipt); unrelated.close();
    await delay(100);
    assert.equal(ui.status().find(s => s.request.id === rejected.id)?.state, 'failed');
    ui.close(); ui = managementClient(join(dir, 'client'), url, secret, () => {}); await ui.ready;
    assert.equal(ui.status().find(s => s.request.id === rejected.id)?.state, 'failed');
    const stop = message('stop', 'reconnect-host', agent, 1);
    const target = [...server.clients].at(-1);
    assert.ok(target);
    const send = target.send.bind(target);
    target.send = ((data: Parameters<typeof target.send>[0], ...args: unknown[]) => {
      const m = open(JSON.parse(String(data)), secret);
      if (m.type === 'receipt' && m.body.operation === stop.id) {
        assert.equal(journal().phase, 'stopped', 'loss really occurs after host commit');
        target.terminate(); return;
      }
      Reflect.apply(send, target, [data, ...args]);
    }) as typeof target.send;
    ui.submit(stop);
    await until(() => journal().phase === 'stopped');
    await until(() => ui.status().find(s => s.request.id === stop.id)?.state === 'completed');
    assert.equal(journal().phase, 'stopped'); assert.equal(journal().revision, 2);
    assert.ok(history().length >= before);
  } finally {
    ui.close(); await h.close(); for (const socket of server.clients) socket.terminate();
    await new Promise<void>(resolve => server.close(() => resolve()));
    rmSync(dir, { recursive: true, force: true });
  }
});

test('process exit before network effects leaves an exact prepared envelope; corrupt file fails closed', async () => {
  const { spawnSync } = await import('node:child_process');
  const { readdirSync } = await import('node:fs');
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-crash-')));
  const secret = newKey();
  const server = await relay(0, publicKey(secret), join(dir, 'relay.json'));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`;
  const root = join(dir, 'client'); const request = message('start', 'offline-host', publicKey(newKey()));
  const child = spawnSync(process.execPath, ['--input-type=module', '-e', `
    import { managementClient } from ${JSON.stringify(new URL('../src/intents.ts', import.meta.url).href)};
    const client = managementClient(${JSON.stringify(root)}, ${JSON.stringify(url)}, ${JSON.stringify(secret)}, () => {});
    client.submit(${JSON.stringify(request)});
    process.exit(23); // No close, no event-loop turn, no websocket publication.
  `], { encoding: 'utf8' });
  assert.equal(child.status, 23, child.stderr);
  const scope = readdirSync(root)[0]!;
  const intentPath = join(root, scope, `${request.id}.intent`);
  const prepared = JSON.parse(readFileSync(intentPath, 'utf8')).envelope;
  let ui = managementClient(root, url, secret, () => {});
  try {
    await ui.ready;
    await until(() => ui.status()[0]?.publication === 'relay observed; not host admission');
    const stored = JSON.parse(readFileSync(join(dir, 'relay.json'), 'utf8'));
    assert.deepEqual(stored.filter((e: unknown) => open(e, secret).type !== 'inspect'), [prepared], 'exact signed/encrypted event reused');
    const sender = connect(url, secret, () => {}); await sender.ready;
    sender.send(message('receipt', 'unrelated-host', request.agent, 1, { operation: request.id, fingerprint: 'f'.repeat(64), result: 'accepted' }));
    await delay(80); sender.close();
    assert.equal(ui.status()[0]?.state, 'pending');
    for (const socket of server.clients) socket.close(1008, 'Policy refusal');
    await until(() => ui.status()[0]?.state === 'unknown');
    ui.close();
    let republished = 0;
    server.on('connection', socket => socket.on('message', data => { if (open(JSON.parse(String(data)), secret).type !== 'inspect') republished++; }));
    ui = managementClient(root, url, secret, () => {}); await ui.ready;
    await delay(150);
    assert.equal(ui.status()[0]?.state, 'unknown');
    assert.equal(republished, 0, 'policy refusal is not blindly retried on reopen');
    ui.close();
    writePrivate(intentPath, { scope, envelope: { ...prepared, signature: '0'.repeat(128) } });
    assert.throws(() => managementClient(root, url, secret, () => {}), /signature/);
  } finally {
    ui.close(); for (const socket of server.clients) socket.terminate();
    await new Promise<void>(resolve => server.close(() => resolve()));
    rmSync(dir, { recursive: true, force: true });
  }
});
