import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, realpathSync, readFileSync, rmSync, unlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { provisionSetup } from './provision.ts';
import { host } from '../src/host.ts';
import { relay } from '../src/relay.ts';
import { connect } from '../src/client.ts';
import { message, newKey, publicKey, type Message } from '../src/protocol.ts';

test('Restart preserves identity, preflights before Stop, replays once and obeys same-batch Stop fencing', async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-restart-')));
  const secret = newKey(), agentSecret = newKey(), agent = publicKey(agentSecret);
  provisionSetup(join(dir, 'setup.json'), { host: 'restart-host', ownerSecret: secret, agentSecret, runner: realpathSync(process.execPath), args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' });
  const server = await relay(0, publicKey(secret), join(dir, 'relay.json'));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`;
  const h = await host(dir, url); const seen: Message[] = [];
  const client = connect(url, secret, m => seen.push(m)); await client.ready;
  const journal = () => JSON.parse(readFileSync(join(dir, 'journal.json'), 'utf8'));
  async function receipt(m: Message) {
    client.send(m);
    for (let i = 0; i < 200; i++) {
      const r = seen.find(r => r.type === 'receipt' && r.body.operation === m.id);
      if (r) return r;
      await delay(25);
    }
    throw Error(`Missing receipt for ${m.type}`);
  }
  try {
    assert.equal((await receipt(message('start', 'restart-host', agent, 0))).body.result, 'accepted');
    const original = journal();
    const setup = readFileSync(join(dir, 'setup.json'));
    unlinkSync(join(dir, 'setup.json'));
    const rejected = await receipt(message('restart', 'restart-host', agent, 1));
    assert.match(String(rejected.body.result), /key\/setup missing/);
    assert.deepEqual(journal().actual, original.actual);
    assert.equal(journal().phase, 'running');
    assert.equal(journal().revision, 1);
    // Only this local test operator restores the file; the host never does.
    writeFileSync(join(dir, 'setup.json'), setup, { mode: 0o600 });
    const restart = message('restart', 'restart-host', agent, 1);
    assert.equal((await receipt(restart)).body.result, 'accepted');
    assert.equal(journal().actual.run, restart.id);
    assert.deepEqual(journal().binding, original.binding);
    assert.deepEqual(journal().assignment, original.assignment);
    const after = readFileSync(join(dir, 'journal.json'), 'utf8');
    client.send(restart); await delay(100);
    assert.equal(readFileSync(join(dir, 'journal.json'), 'utf8'), after);
    const cancelled = message('restart', 'restart-host', agent, 2);
    client.send(cancelled);
    assert.equal((await receipt(message('stop', 'restart-host', agent, 2))).body.result, 'accepted');
    assert.match(journal().operations[cancelled.id].reply.body.result, /cancelled/);
    assert.equal(journal().actual, null); assert.equal(journal().phase, 'stopped');
    await delay(150); assert.equal(journal().actual, null);
  } finally {
    await h.close(); client.close(); for (const c of server.clients) c.terminate();
    await new Promise<void>(r => server.close(() => r())); rmSync(dir, { recursive: true, force: true });
  }
});

test('ACP Restart preflight rejection preserves actual run; authorized Stop interrupts delayed probe without revival', async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-restart-acp-')));
  const secret = newKey(), agentSecret = newKey(), agent = publicKey(agentSecret);
  const wrapper = join(dir, 'fixture.mjs');
  writeFileSync(wrapper, `import { readFileSync } from 'node:fs'; process.argv[2] = readFileSync('mode', 'utf8'); await import(${JSON.stringify(new URL(`file://${resolve('test/acp-fixture.ts')}`).href)});`);
  writeFileSync(join(dir, 'mode'), 'ok');
  provisionSetup(join(dir, 'setup.json'), { host: 'acp-restart', ownerSecret: secret, agentSecret, runner: realpathSync(process.execPath), args: [wrapper], workspace: dir, mode: 'buzz-agent-databricks-v2', serviceHome: dir, configDirectory: dir, databricksHost: 'https://fixture.invalid' });
  const server = await relay(0, publicKey(secret), join(dir, 'relay.json'));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`, seen: Message[] = [];
  const h = await host(dir, url), client = connect(url, secret, m => seen.push(m)); await client.ready;
  const journal = () => JSON.parse(readFileSync(join(dir, 'journal.json'), 'utf8'));
  async function until(predicate: () => boolean) {
    for (let i = 0; i < 200; i++) { if (predicate()) return; await delay(25); }
    throw Error('Missing ACP Restart evidence');
  }
  async function request(m: Message) { client.send(m); await until(() => seen.some(r => r.type === 'receipt' && r.body.operation === m.id)); return journal().operations[m.id].reply.body.result; }
  try {
    assert.equal(await request(message('start', 'acp-restart', agent, 0)), 'accepted');
    const original = journal().actual;
    writeFileSync(join(dir, 'mode'), 'reject');
    assert.notEqual(await request(message('restart', 'acp-restart', agent, 1)), 'accepted');
    assert.deepEqual(journal().actual, original); assert.equal(journal().phase, 'running');
    writeFileSync(join(dir, 'mode'), 'delayed');
    const restart = message('restart', 'acp-restart', agent, 1); client.send(restart);
    await until(() => { try { return readFileSync(join(dir, 'prompt-started'), 'utf8') === '1'; } catch { return false; } });
    // Invalid retractions cannot interrupt the prerequisite probe or old run.
    client.send(message('stop', 'acp-restart', publicKey(newKey()), 1));
    client.send(message('stop', 'acp-restart', agent, 99));
    client.send(message('stop', 'acp-restart', agent, 1, { invalid: true }));
    await delay(100); assert.deepEqual(journal().actual, original);
    const began = Date.now();
    assert.equal(await request(message('stop', 'acp-restart', agent, 1)), 'accepted');
    assert.ok(Date.now() - began < 2500, 'Stop interrupts four-second preflight');
    assert.notEqual(journal().operations[restart.id].reply.body.result, 'accepted');
    assert.equal(journal().actual, null); assert.equal(journal().phase, 'stopped');
    await delay(4200); assert.equal(journal().actual, null);
  } finally {
    await h.close(); client.close(); for (const c of server.clients) c.terminate();
    await new Promise<void>(r => server.close(() => r())); rmSync(dir, { recursive: true, force: true });
  }
});
