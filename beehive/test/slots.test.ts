import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, type ChildProcess } from 'node:child_process';
import { once } from 'node:events';
import { mkdtempSync, realpathSync, rmSync, readFileSync, existsSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { setTimeout as delay } from 'node:timers/promises';
import { createGenesis } from '../src/assignment.ts';
import { provision, host, type Setup } from '../src/host.ts';
import { migrateSlots, addSlot } from '../src/slots.ts';
import { newKey, publicKey, message, type Message } from '../src/protocol.ts';
import { writePrivate } from '../src/storage.ts';
import { relay } from '../src/relay.ts';
import { connect } from '../src/client.ts';
import { managementClient } from '../src/intents.ts';

async function until(predicate: () => boolean) {
  for (let i = 0; i < 400; i++) { if (predicate()) return; await delay(20); }
  throw Error('Missing single-installation evidence');
}
async function closeChild(child: ChildProcess) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const done = once(child, 'exit'); child.kill('SIGTERM'); await done;
}

test('one real host process/connection: X and Y TUI lifecycle, independent receipts, reconnect, restart, standby denial', { timeout: 30000 }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-slots-')));
  const ownerSecret = newKey(), x = newKey(), y = newKey(); const X = publicKey(x), Y = publicKey(y);
  const source = join(dir, 'source'); const target = join(dir, 'target');
  const setup: Setup = { host: 'source', ownerSecret, agentSecret: x, runner: process.execPath, args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' };
  const gx = createGenesis(publicKey(ownerSecret), X, 'source');
  provision(source, setup, gx);
  const before = readFileSync(join(source, 'journal.json'));
  migrateSlots(source); assert.deepEqual(readFileSync(join(source, 'journal.json')), before);
  addSlot(source, y, createGenesis(publicKey(ownerSecret), Y, 'source'));
  const manifest = JSON.parse(readFileSync(join(source, 'setup.json'), 'utf8'));
  assert.equal(Object.keys(manifest.setups).length, 1); assert.equal(Object.keys(manifest.agents).length, 2);
  assert.equal(existsSync(join(source, 'agents', Y, 'setup.json')), false, 'no second daemon setup/key copy');
  provision(target, { ...setup, host: 'standby' }, gx);
  writePrivate(join(dir, 'identity.json'), { secret: ownerSecret });
  const server = await relay(0, publicKey(ownerSecret), join(dir, 'relay.json'));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`; const seen: Message[] = []; const children: ChildProcess[] = [];
  const ui = connect(url, ownerSecret, m => seen.push(m)); await ui.ready;
  const journal = (agent: string) => JSON.parse(readFileSync(agent === X ? join(source, 'journal.json') : join(source, 'agents', agent, 'journal.json'), 'utf8'));
  async function launch(path: string) {
    const child = spawn(process.execPath, ['src/cli.ts', 'host', path, url], { stdio: ['ignore', 'pipe', 'pipe'] }); children.push(child);
    let output = ''; child.stdout?.on('data', chunk => { output += chunk.toString(); }); child.stderr?.resume();
    await until(() => output.includes('Host online')); return child;
  }
  async function receipt(m: Message) { ui.send(m); await until(() => seen.some(r => r.type === 'receipt' && r.body.operation === m.id && r.agent === m.agent)); return seen.find(r => r.type === 'receipt' && r.body.operation === m.id && r.agent === m.agent)!; }
  try {
    let processHost = await launch(source);
    await until(() => seen.some(m => m.type === 'inventory' && m.agent === Y));
    assert.equal(server.clients.size, 2, 'exactly ONE host management socket plus observation client');
    assert.equal(existsSync(join(source, 'host.lock')), true);
    await assert.rejects(host(source, url), /EEXIST/);
    assert.throws(() => addSlot(source, newKey(), gx), /EEXIST/);
    // Actual remote TUI selects numbered slots, saves and starts both, then stops X.
    const terminal = spawn(process.execPath, ['src/cli.ts', 'tui', join(dir, 'identity.json'), url], { stdio: ['pipe', 'pipe', 'pipe'] }); children.push(terminal);
    let output = ''; let index = 0;
    const steps: { prompt: string; value: string; gate?: () => boolean }[] = [
      { prompt: 'beehive> ', value: 'hosts' }, { prompt: 'beehive> ', value: 'select source' },
      { prompt: 'beehive> ', value: 'select 1' }, { prompt: 'beehive> ', value: 'save' },
      { prompt: 'Model: ', value: 'fixture-model' }, { prompt: 'Workspace: ', value: dir }, { prompt: 'Behavior profile: ', value: 'default' },
      { prompt: 'beehive> ', value: 'start', gate: () => journal(X).revision === 1 },
      { prompt: 'beehive> ', value: 'select 2', gate: () => journal(X).phase === 'running' },
      { prompt: 'beehive> ', value: 'save' }, { prompt: 'Model: ', value: 'fixture-model' }, { prompt: 'Workspace: ', value: dir }, { prompt: 'Behavior profile: ', value: 'default' },
      { prompt: 'beehive> ', value: 'start', gate: () => journal(Y).revision === 1 },
      { prompt: 'beehive> ', value: 'show', gate: () => journal(Y).phase === 'running' },
      { prompt: 'beehive> ', value: 'select 1' }, { prompt: 'beehive> ', value: 'stop' },
      { prompt: 'beehive> ', value: 'agents', gate: () => journal(X).phase === 'stopped' }, { prompt: 'beehive> ', value: 'quit' },
    ];
    let pending = false; let cursor = 0;
    terminal.stdout.on('data', chunk => {
      output += chunk.toString(); const step = steps[index];
      if (pending || !step || !output.slice(cursor).includes(step.prompt)) return;
      pending = true;
      void (async () => { if (step.gate) await until(step.gate); await delay(100); index++; cursor = output.length; pending = false; terminal.stdin.write(`${step.value}\n`); })();
    }); terminal.stderr.resume();
    const [code] = await once(terminal, 'exit'); assert.equal(code, 0);
    assert.match(output, /Unknown or ambiguous host/);
    assert.match(output, new RegExp(`Selected source agent ${X}`)); assert.match(output, new RegExp(`Selected source agent ${Y}`));
    assert.match(output, /"phase": "running"/);
    assert.equal(journal(X).revision, 3); assert.equal(journal(Y).revision, 2); assert.equal(journal(Y).phase, 'running');
    const yBefore = readFileSync(join(source, 'agents', Y, 'journal.json'));
    const previous = Object.values(journal(Y).operations).map((o: any) => o.reply).find((r: Message) => r.body.result === 'accepted') as Message;
    assert.ok(previous);
    // Same operation ID on a different slot is not a receipt reservation conflict.
    const stopX = message('stop', 'source', X, 3); stopX.id = String(previous.body.operation);
    assert.equal((await receipt(stopX)).body.result, 'accepted');
    assert.deepEqual(readFileSync(join(source, 'agents', Y, 'journal.json')), yBefore, 'Stop X cannot mutate Y receipt/revision/run');
    assert.equal((await receipt(message('stop', 'source', publicKey(newKey()), 0))).body.result, 'not-authority');
    ui.send(message('stop', 'wrong-host', Y, 2)); await delay(100); assert.equal(journal(Y).phase, 'running');
    // Drop only the host's socket. Both slots replay receipts through ONE replacement.
    const hostSocket = [...server.clients][1]!; seen.length = 0; hostSocket.terminate();
    await until(() => seen.some(m => m.type === 'receipt' && m.body.operation === previous.body.operation));
    await until(() => seen.some(m => m.type === 'inventory' && m.agent === Y && m.body.phase === 'running'));
    assert.equal(server.clients.size, 2); assert.deepEqual(readFileSync(join(source, 'agents', Y, 'journal.json')), yBefore);
    await launch(target);
    assert.equal((await receipt(message('start', 'standby', X, 0))).body.result, 'not-authority');
    await closeChild(processHost); assert.equal(journal(Y).phase, 'stopped'); assert.equal(journal(Y).revision, 2);
    seen.length = 0; processHost = await launch(source);
    await until(() => seen.some(m => m.type === 'inventory' && m.agent === Y && m.host === 'source' && m.body.phase === 'stopped'));
    assert.equal(journal(X).assignment.assignedHost, 'source'); assert.equal(journal(Y).assignment.assignedHost, 'source');
    assert.equal((await receipt(message('start', 'source', Y, 2))).body.result, 'accepted');
    assert.equal((await receipt(message('start', 'standby', X, 0))).body.result, 'not-authority');
    console.log(`Single host PID ${processHost.pid}; identities ${X}, ${Y}; one connection, shared setup, independent journals`);
  } finally {
    for (const child of children) await closeChild(child);
    ui.close(); for (const c of server.clients) c.terminate();
    await new Promise<void>(resolve => server.close(() => resolve())); rmSync(dir, { recursive: true, force: true });
  }
});

test('per-slot concurrent ACP admission: cancelling X leaves pending Y independent and receipts replay per agent', { timeout: 20000 }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-slots-cancel-')));
  const ownerSecret = newKey(), x = newKey(), y = newKey(); const X = publicKey(x), Y = publicKey(y);
  const setup: Setup = { host: 'both', ownerSecret, agentSecret: x, runner: process.execPath, args: [resolve('test/acp-fixture.ts'), 'delayed'], workspace: dir, mode: 'buzz-agent-databricks-v2', serviceHome: dir, configDirectory: dir, databricksHost: 'https://fixture.invalid' };
  provision(dir, setup, createGenesis(publicKey(ownerSecret), X, 'both')); migrateSlots(dir); addSlot(dir, y, createGenesis(publicKey(ownerSecret), Y, 'both'));
  const server = await relay(0, publicKey(ownerSecret), join(dir, 'relay.json')); const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`; const h = await host(dir, url); const seen: Message[] = [];
  const client = connect(url, ownerSecret, m => seen.push(m)); await client.ready;
  const journal = (a: string) => JSON.parse(readFileSync(a === X ? join(dir, 'journal.json') : join(dir, 'agents', a, 'journal.json'), 'utf8'));
  const startX = message('start', 'both', X, 0), startY = message('start', 'both', Y, 0); startY.id = startX.id;
  try {
    client.send(startX); client.send(startY);
    await until(() => journal(X).phase === 'transitioning' && journal(Y).phase === 'transitioning');
    const stopX = message('stop', 'both', X, 0); client.send(stopX);
    await until(() => seen.some(m => m.type === 'receipt' && m.body.operation === stopX.id));
    assert.equal(journal(X).phase, 'stopped'); assert.equal(journal(Y).phase, 'transitioning');
    await until(() => journal(Y).phase === 'running');
    assert.match(journal(X).operations[startX.id].reply.body.result, /cancelled/);
    assert.equal(journal(Y).operations[startY.id].reply.body.result, 'accepted'); assert.equal(journal(Y).revision, 1);
    const beforeX = readFileSync(join(dir, 'journal.json')), beforeY = readFileSync(join(dir, 'agents', Y, 'journal.json'));
    seen.length = 0; client.send(startX); client.send(startY);
    await until(() => seen.filter(m => m.type === 'receipt' && m.body.operation === startX.id).length >= 2);
    assert.deepEqual(readFileSync(join(dir, 'journal.json')), beforeX); assert.deepEqual(readFileSync(join(dir, 'agents', Y, 'journal.json')), beforeY);
    // Close tears down the remaining running sibling independently.
    await h.close(); assert.equal(journal(Y).phase, 'stopped'); assert.equal(existsSync(join(dir, 'host.lock')), false);
  } finally {
    await h.close(); client.close(); for (const c of server.clients) c.terminate();
    await new Promise<void>(resolve => server.close(() => resolve())); rmSync(dir, { recursive: true, force: true });
  }
});

test('explicit upgrade preserves real receipts; uncertain first slot cannot skip healthy sibling teardown', { timeout: 15000 }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-slots-close-')));
  const ownerSecret = newKey(), x = newKey(), y = newKey(); const X = publicKey(x), Y = publicKey(y);
  provision(dir, { host: 'close', ownerSecret, agentSecret: x, runner: process.execPath, args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' }, createGenesis(publicKey(ownerSecret), X, 'close'));
  const server = await relay(0, publicKey(ownerSecret), join(dir, 'relay.json')); const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`; let h = await host(dir, url); const seen: Message[] = [];
  const client = connect(url, ownerSecret, m => seen.push(m)); await client.ready;
  async function request(m: Message) { client.send(m); await until(() => seen.some(r => r.type === 'receipt' && r.body.operation === m.id)); }
  let expectedFailure = false;
  try {
    await request(message('start', 'close', X, 0)); await request(message('stop', 'close', X, 1)); await h.close();
    const journalPath = join(dir, 'journal.json'); const before = readFileSync(journalPath);
    migrateSlots(dir); assert.deepEqual(readFileSync(journalPath), before);
    const migrated = JSON.parse(before.toString()); assert.equal(migrated.revision, 2); assert.equal(Object.keys(migrated.operations).length, 2); assert.equal(migrated.outbox.length, 2);
    assert.throws(() => migrateSlots(dir), /already enabled/);
    addSlot(dir, y, createGenesis(publicKey(ownerSecret), Y, 'close'));
    // Explicitly simulate recovered unknown execution, with no invented live PID.
    writePrivate(journalPath, { ...migrated, phase: 'quarantined' });
    h = await host(dir, url);
    await request(message('start', 'close', Y, 0));
    const yPath = join(dir, 'agents', Y, 'journal.json');
    assert.equal(JSON.parse(readFileSync(yPath, 'utf8')).phase, 'running');
    expectedFailure = true; await assert.rejects(h.close(), /Incomplete owned teardown/);
    assert.equal(JSON.parse(readFileSync(journalPath, 'utf8')).phase, 'quarantined');
    assert.equal(JSON.parse(readFileSync(yPath, 'utf8')).phase, 'stopped');
    assert.equal(JSON.parse(readFileSync(yPath, 'utf8')).actual, null);
    assert.equal(existsSync(join(dir, 'host.lock')), true, 'uncertain sibling retains installation fence');
  } finally {
    if (!expectedFailure) await h.close();
    client.close(); for (const c of server.clients) c.terminate();
    await new Promise<void>(resolve => server.close(() => resolve())); rmSync(dir, { recursive: true, force: true });
  }
});

test('management reconciliation queries every unresolved host/agent slot, then resolves both from real host', { timeout: 15000 }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-slots-intents-')));
  const owner = newKey(), x = newKey(), y = newKey(); const X = publicKey(x), Y = publicKey(y);
  const source = join(dir, 'host');
  provision(source, { host: 'shared', ownerSecret: owner, agentSecret: x, runner: process.execPath, args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' }, createGenesis(publicKey(owner), X, 'shared'));
  migrateSlots(source); addSlot(source, y, createGenesis(publicKey(owner), Y, 'shared'));
  const server = await relay(0, publicKey(owner), join(dir, 'relay.json')); const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`; const seen: Message[] = [];
  const observer = connect(url, owner, m => seen.push(m)); await observer.ready;
  let client = managementClient(join(dir, 'intents'), url, owner, () => {}, () => {}); await client.ready;
  let h: Awaited<ReturnType<typeof host>> | undefined;
  try {
    client.submit(message('start', 'shared', X, 0)); client.submit(message('start', 'shared', Y, 0));
    await until(() => seen.filter(m => m.type === 'start').length >= 2);
    client.close(); seen.length = 0;
    client = managementClient(join(dir, 'intents'), url, owner, () => {}, () => {}); await client.ready;
    // Scope assertion is against the new connection's actual inspect publications,
    // not historical relay replay: inspect IDs are fresh, unlike retained intents.
    await delay(100); const prior = new Set(seen.map(m => m.id)); client.reconcile();
    await until(() => [X, Y].every(a => seen.some(m => m.type === 'inspect' && m.agent === a && !prior.has(m.id))));
    h = await host(source, url);
    await until(() => client.status().every(i => i.state === 'completed'));
    assert.equal(client.status().length, 2);
  } finally {
    await h?.close(); client.close(); observer.close(); for (const c of server.clients) c.terminate();
    await new Promise<void>(resolve => server.close(() => resolve())); rmSync(dir, { recursive: true, force: true });
  }
});
