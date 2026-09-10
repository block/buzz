import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, type ChildProcess } from 'node:child_process';
import { once } from 'node:events';
import { mkdtempSync, realpathSync, rmSync, readFileSync, unlinkSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { setTimeout as delay } from 'node:timers/promises';
import { createGenesis } from '../src/assignment.ts';
import { provision, host, migrateAssignment, type Setup } from '../src/host.ts';
import { newKey, publicKey, message, type Message } from '../src/protocol.ts';
import { writePrivate } from '../src/storage.ts';
import { relay } from '../src/relay.ts';
import { connect } from '../src/client.ts';

test('real host processes: same key on standby never grants Start; second identity and TUI routing', { timeout: 25000 }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-assignment-')));
  const ownerSecret = newKey(), x = newKey(), y = newKey();
  const gx = createGenesis(publicKey(ownerSecret), publicKey(x), 'source');
  const gy = createGenesis(publicKey(ownerSecret), publicKey(y), 'other');
  const setup = (name: string, agentSecret: string): Setup => ({ host: name, ownerSecret, agentSecret, runner: process.execPath, args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' });
  provision(join(dir, 'source'), setup('source', x), gx);
  provision(join(dir, 'target'), setup('target', x), gx);
  provision(join(dir, 'other'), setup('other', y), gy);
  writePrivate(join(dir, 'identity.json'), { secret: ownerSecret });
  const server = await relay(0, publicKey(ownerSecret), join(dir, 'relay.json'));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`;
  const children: ChildProcess[] = []; const seen: Message[] = [];
  const ui = connect(url, ownerSecret, m => seen.push(m)); await ui.ready;
  async function wait(predicate: (m: Message) => boolean) {
    for (let i = 0; i < 240; i++) { const m = seen.find(predicate); if (m) return m; await delay(20); }
    throw Error('Missing assignment boundary evidence');
  }
  async function startHost(name: string) {
    const child = spawn(process.execPath, ['src/cli.ts', 'host', join(dir, name), url], { stdio: ['ignore', 'pipe', 'pipe'] });
    children.push(child); child.stdout?.resume(); child.stderr?.resume();
    await wait(m => m.type === 'inventory' && m.host === name); return child;
  }
  async function closeHost(child: ChildProcess) {
    if (child.exitCode !== null) return;
    const done = once(child, 'exit'); child.kill('SIGTERM'); await done;
  }
  async function request(type: 'start' | 'stop' | 'save', name: string, agent: string, revision: number, body = {}) {
    const m = message(type, name, agent, revision, body); ui.send(m);
    return wait(r => r.type === 'receipt' && r.body.operation === m.id);
  }
  try {
    let source = await startHost('source'); await startHost('target'); await startHost('other');
    assert.equal((await request('start', 'target', publicKey(x), 0)).body.result, 'not-authority');
    assert.equal((await request('start', 'source', publicKey(y), 0)).body.result, 'not-authority');
    assert.equal((await request('start', 'source', publicKey(x), 0)).body.result, 'accepted');
    assert.equal((await request('start', 'other', publicKey(y), 0)).body.result, 'accepted');
    assert.equal((await request('stop', 'source', publicKey(x), 1)).body.result, 'accepted');
    await closeHost(source); seen.length = 0; source = await startHost('source');
    const stopped = await wait(m => m.type === 'inventory' && m.host === 'source' && m.revision === 2);
    assert.equal(stopped.body.assignedHost, 'source'); assert.equal(stopped.body.phase, 'stopped');
    assert.equal((await request('start', 'target', publicKey(x), 0)).body.result, 'not-authority');
    assert.equal((await request('start', 'source', publicKey(x), 2)).body.result, 'accepted');
    // Actual TUI subprocess selects distinct identities, without a singleton key cache.
    const terminal = spawn(process.execPath, ['src/cli.ts', 'tui', join(dir, 'identity.json'), url], { stdio: ['pipe', 'pipe', 'pipe'] });
    children.push(terminal); let output = ''; let index = 0;
    const commands = ['agents', 'hosts', 'select target', 'show', 'select other', 'show', 'quit'];
    terminal.stdout.on('data', chunk => {
      output += chunk.toString();
      if (output.endsWith('beehive> ') && index < commands.length) setTimeout(() => terminal.stdin.write(`${commands[index++]}\n`), 100);
    });
    terminal.stderr.resume(); const [code] = await once(terminal, 'exit'); assert.equal(code, 0);
    assert.match(output, new RegExp(`Selected target agent ${publicKey(x)}`));
    assert.match(output, new RegExp(`Selected other agent ${publicKey(y)}`));
    assert.match(output, /"executionAuthority": false/); assert.match(output, /"executionAuthority": true/);
    // No reachable source is not an excuse to admit the provisioned standby.
    await closeHost(source);
    assert.equal((await request('start', 'target', publicKey(x), 0)).body.result, 'not-authority');
    assert.equal((await request('stop', 'other', publicKey(y), 1)).body.result, 'accepted');
    // Missing local key/setup is not reconstructed from hydrated memory.
    assert.equal((await request('start', 'other', publicKey(y), 2)).body.result, 'accepted');
    unlinkSync(join(dir, 'other', 'setup.json'));
    assert.equal((await request('stop', 'other', publicKey(y), 3)).body.result, 'accepted');
    assert.match(String((await request('start', 'other', publicKey(y), 4)).body.result), /Local agent key\/setup missing/);
    assert.throws(() => readFileSync(join(dir, 'other', 'setup.json')));
    const targetState = JSON.parse(readFileSync(join(dir, 'target', 'journal.json'), 'utf8'));
    assert.equal(targetState.assignment.assignedHost, 'source'); assert.equal(targetState.actual, null); assert.equal(targetState.revision, 0);
  } finally {
    for (const child of children) await closeHost(child);
    ui.close(); for (const c of server.clients) c.terminate();
    await new Promise<void>(resolve => server.close(() => resolve())); rmSync(dir, { recursive: true, force: true });
  }
});

test('missing journals fail closed; explicit stopped legacy migration preserves history and rejects replacement', async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-migration-')));
  const ownerSecret = newKey(), agentSecret = newKey();
  const setup: Setup = { host: 'legacy', ownerSecret, agentSecret, runner: process.execPath, args: [], workspace: dir, mode: 'fixture' };
  const genesis = createGenesis(publicKey(ownerSecret), publicKey(agentSecret), setup.host);
  try {
    provision(dir, setup, genesis);
    const path = join(dir, 'journal.json'); const state = JSON.parse(readFileSync(path, 'utf8'));
    delete state.assignment; state.revision = 17; writePrivate(path, state);
    await assert.rejects(host(dir, 'ws://127.0.0.1:1'), /explicit local migrate-assignment/);
    migrateAssignment(dir, genesis);
    assert.equal(JSON.parse(readFileSync(path, 'utf8')).revision, 17);
    assert.throws(() => migrateAssignment(dir, genesis), /already pinned/);
    unlinkSync(path);
    await assert.rejects(host(dir, 'ws://127.0.0.1:1'), /Authority journal missing/);
    assert.throws(() => migrateAssignment(dir, genesis), /ENOENT/);
    assert.throws(() => provision(dir, setup, genesis), /cannot reset authority/);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
