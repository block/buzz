import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, realpathSync, readFileSync, writeFileSync, rmSync, unlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { Profiles, profile, profileRevision } from '../src/profiles.ts';
import { selection } from '../src/handoff.ts';
import { message, newKey, publicKey, open, seal, type Message } from '../src/protocol.ts';
import { provisionSetup } from './provision.ts';
import { host } from '../src/host.ts';
import { relay } from '../src/relay.ts';
import { connect } from '../src/client.ts';
import { managementClient } from '../src/intents.ts';

function version(instructions: string, parent: string | null = null) {
  return profile({ name: 'Careful', parent, instructions, revision: profileRevision('Careful', parent, instructions) });
}
async function until(predicate: () => boolean) {
  for (let i = 0; i < 400; i++) { if (predicate()) return; await delay(20); }
  throw Error('Profile evidence missing');
}

test('one profile codec: immutable branches, missing lineage, hashes, invalid references, no launch fields', () => {
  const a = version('A'), b = version('B', a.revision), c = version('C', a.revision);
  const catalog = new Profiles();
  catalog.receive(message('profile', 'profiles', 'profiles', 0, b));
  assert.throws(() => catalog.resolve(b.revision), /incomplete/);
  catalog.receive(message('profile', 'profiles', 'profiles', 0, a));
  catalog.receive(message('profile', 'profiles', 'profiles', 0, c));
  assert.equal(catalog.list().length, 3);
  assert.equal(catalog.resolve(b.revision).instructions, 'B');
  assert.equal(catalog.resolve(c.revision).instructions, 'C');
  assert.throws(() => profile({ ...a, model: 'injected' }), /fields/);
  assert.throws(() => version('x'.repeat(2049)), /instructions/);
  assert.throws(() => seal(message('save', 'h', 'a', 0, { oversized: 'x'.repeat(32768) }), newKey()), /wire size/);
  assert.throws(() => profile({ ...a, instructions: 'tampered' }), /revision conflict/);
  assert.throws(() => selection({ model: 'm', workspace: '/tmp', profile: a.revision }), /reference/);
  assert.throws(() => selection({ model: 'm', workspace: '/tmp', profile: 'default', behavior: a }), /Default/);
  const copy = catalog.resolve(a.revision); copy.instructions = 'changed';
  assert.equal(catalog.resolve(a.revision).instructions, 'A');
});

test('profile publication and Save racing async Restart never mix captured inputs; Stop cancels without revival', { timeout: 30000 }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-profiles-')));
  const secret = newKey(), agentSecret = newKey(), agent = publicKey(agentSecret);
  const wrapper = join(dir, 'fixture.mjs');
  writeFileSync(wrapper, `import { readFileSync, appendFileSync } from 'node:fs'; appendFileSync('received.jsonl', JSON.stringify({instructions:process.env.BUZZ_AGENT_SYSTEM_PROMPT,pid:process.pid})+'\\n'); process.argv[2] = readFileSync('mode', 'utf8'); await import(${JSON.stringify(new URL(`file://${resolve('test/acp-fixture.ts')}`).href)});`);
  writeFileSync(join(dir, 'mode'), 'ok');
  provisionSetup(join(dir, 'setup.json'), { host: 'profiles', ownerSecret: secret, agentSecret, runner: realpathSync(process.execPath), args: [wrapper], workspace: dir, mode: 'buzz-agent-databricks-v2', serviceHome: dir, configDirectory: dir, databricksHost: 'https://fixture.invalid' });
  const server = await relay(0, publicKey(secret), join(dir, 'relay.json'));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`, seen: Message[] = [];
  const h = await host(dir, url), client = connect(url, secret, m => seen.push(m)); await client.ready;
  const journal = () => JSON.parse(readFileSync(join(dir, 'journal.json'), 'utf8'));
  const chosen = (p: ReturnType<typeof version>) => ({ model: 'databricks-claude-haiku-4-5', workspace: dir, profile: p.revision, behavior: p });
  async function request(m: Message) { client.send(m); await until(() => seen.some(r => r.type === 'receipt' && r.body.operation === m.id)); return journal().operations[m.id].reply.body.result; }
  async function publish(p: ReturnType<typeof version>) { const m = message('profile', 'profiles', 'profiles', 0, p); client.send(m); await until(() => seen.some(r => r.id === m.id)); }
  try {
    const a = version('First instructions'), b = version('Second instructions', a.revision), c = version('Third instructions', b.revision);
    assert.match(await request(message('save', 'profiles', agent, 0, chosen(a))), /unavailable/);
    await publish(a);
    assert.equal(await request(message('save', 'profiles', agent, 0, chosen(a))), 'saved; running configuration unchanged');
    assert.equal(await request(message('start', 'profiles', agent, 1)), 'accepted');
    const original = journal().actual;
    await publish(b); assert.deepEqual(journal().actual, original);
    assert.equal(await request(message('save', 'profiles', agent, 2, chosen(b))), 'saved; running configuration unchanged');
    assert.deepEqual(journal().actual, original);
    assert.equal(await request(message('save', 'profiles', agent, 2, chosen(a))), 'revision-conflict');
    writeFileSync(join(dir, 'mode'), 'delayed');
    const restart = message('restart', 'profiles', agent, 3); client.send(restart);
    await until(() => { try { return readFileSync(join(dir, 'prompt-started'), 'utf8') === '1'; } catch { return false; } });
    await publish(c);
    const racingSave = message('save', 'profiles', agent, 3, chosen(c)); client.send(racingSave);
    writeFileSync(join(dir, 'mode'), 'ok');
    await until(() => seen.some(r => r.type === 'receipt' && r.body.operation === racingSave.id));
    assert.equal(journal().operations[restart.id].reply.body.result, 'accepted');
    assert.equal(journal().operations[racingSave.id].reply.body.result, 'revision-conflict');
    assert.equal(journal().actual.selection.behavior.instructions, b.instructions);
    assert.equal(journal().actual.appliedInstructions.revision, b.revision);
    assert.notEqual(journal().actual.run, original.run);
    assert.deepEqual(journal().runs[original.run], original);
    assert.notEqual(journal().actual.preparedInputHash, original.preparedInputHash);
    assert.deepEqual(readFileSync(join(dir, 'received.jsonl'), 'utf8').trim().split('\n').map(s => JSON.parse(s).instructions), [a.instructions, b.instructions, b.instructions]);
    assert.equal(await request(message('save', 'profiles', agent, 4, chosen(c))), 'saved; running configuration unchanged');
    unlinkSync(join(dir, 'prompt-started')); writeFileSync(join(dir, 'mode'), 'delayed');
    const cancelled = message('restart', 'profiles', agent, 5); client.send(cancelled);
    await until(() => { try { return readFileSync(join(dir, 'prompt-started'), 'utf8') === '1'; } catch { return false; } });
    assert.equal(await request(message('stop', 'profiles', agent, 5)), 'accepted');
    assert.notEqual(journal().operations[cancelled.id].reply.body.result, 'accepted');
    await delay(4200); assert.equal(journal().actual, null); assert.equal(journal().phase, 'stopped');
    assert.deepEqual(journal().runs[original.run], original);
  } finally {
    await h.close(); client.close(); for (const c of server.clients) c.terminate();
    await new Promise<void>(r => server.close(() => r())); rmSync(dir, { recursive: true, force: true });
  }
});

test('Restart lost receipt before relay storage: real durable client reconcile recovers outbox, no additional launch', async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-restart-receipt-')));
  const secret = newKey(), agentSecret = newKey(), agent = publicKey(agentSecret);
  provisionSetup(join(dir, 'setup.json'), { host: 'receipt', ownerSecret: secret, agentSecret, runner: realpathSync(process.execPath), args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' });
  const server = await relay(0, publicKey(secret), join(dir, 'relay.json'));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`, h = await host(dir, url);
  let client = managementClient(join(dir, 'intents'), url, secret, () => {}); await client.ready;
  const journal = () => JSON.parse(readFileSync(join(dir, 'journal.json'), 'utf8'));
  try {
    client.submit(message('start', 'receipt', agent, 0)); await until(() => journal().phase === 'running');
    const restart = message('restart', 'receipt', agent, 1);
    const socket = [...server.clients][0]!;
    const handlers = socket.listeners('message'); socket.removeAllListeners('message');
    let drop = true, losses = 0;
    socket.on('message', (raw, binary) => {
      const m = open(JSON.parse(String(raw)), secret);
      if (drop && m.type === 'receipt' && m.body.operation === restart.id) { losses++; return; }
      for (const handler of handlers) Reflect.apply(handler, socket, [raw, binary]);
    });
    client.submit(restart); await until(() => losses > 0);
    assert.equal(journal().actual.run, restart.id);
    const snapshot = readFileSync(join(dir, 'journal.json'));
    client.close(); drop = false;
    client = managementClient(join(dir, 'intents'), url, secret, () => {}); await client.ready;
    await until(() => client.status().find(s => s.request.id === restart.id)?.state === 'completed');
    assert.deepEqual(readFileSync(join(dir, 'journal.json')), snapshot);
    await until(() => readFileSync(join(dir, 'received-instructions.jsonl'), 'utf8').trim().split('\n').length === 2);
    await delay(100);
    assert.equal(readFileSync(join(dir, 'received-instructions.jsonl'), 'utf8').trim().split('\n').length, 2);
  } finally {
    client.close(); await h.close(); for (const c of server.clients) c.terminate();
    await new Promise<void>(r => server.close(() => r())); rmSync(dir, { recursive: true, force: true });
  }
});
