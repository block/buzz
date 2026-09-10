import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtempSync, realpathSync, mkdirSync, readFileSync, statSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { newKey, publicKey, message, type Message } from '../src/protocol.ts';
import { readPrivate, writePrivate } from '../src/storage.ts';
import { host, provision } from '../src/host.ts';
import { relay } from '../src/relay.ts';
import { connect } from '../src/client.ts';
import { installationSlots } from '../src/slots.ts';
import { configure } from '../src/configurations.ts';

async function until(check: () => boolean) {
  for (let i = 0; i < 600; i++) { if (check()) return; await delay(20); }
  throw Error('Missing named-configuration evidence');
}
type Step = { prompt: string; answer: string; gate?: () => boolean };
async function terminal(args: string[], steps: Step[]) {
  const child = spawn(process.execPath, ['src/cli.ts', ...args], { stdio: ['pipe', 'pipe', 'pipe'] });
  let output = '', error = '', cursor = 0, index = 0, pending = false;
  child.stderr.on('data', c => { error += c.toString(); });
  let failure: unknown;
  child.stdout.on('data', c => {
    output += c.toString(); const step = steps[index];
    if (!step || pending) return;
    const position = output.indexOf(step.prompt, cursor); if (position < 0) return;
    pending = true;
    void (async () => {
      if (step.gate) await until(step.gate);
      // Receipts and their inventory are distinct publications; wait for the TUI observation.
      await delay(80); cursor = position + step.prompt.length; index++; pending = false;
      child.stdin.write(`${step.answer}\n`);
    })().catch(e => { failure = e; child.kill('SIGTERM'); });
  });
  const timer = setTimeout(() => child.kill('SIGTERM'), 25000);
  try {
    const [code] = await once(child, 'exit');
    if (failure) throw failure;
    assert.equal(code, 0, `${error}\n${output}`); assert.equal(index, steps.length, output);
    return output;
  } finally { clearTimeout(timer); if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM'); }
}

test('real local wizard: two keys share one setup; real TUI named candidates leave runs alone, Restart applies exact selection; standby Save never Start', { timeout: 40000 }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-named-'))), source = join(dir, 'source'), target = join(dir, 'target');
  const workspace = join(dir, 'workspace'); mkdirSync(workspace, { mode: 0o700 });
  const secret = newKey(), identity = join(dir, 'identity.json'); writePrivate(identity, { secret });
  const setupOutput = await terminal(['setup', source, identity], [
    { prompt: 'Host name: ', answer: 'source' },
    { prompt: 'Setup [', answer: '1' },
    { prompt: 'Absolute fixture runner executable: ', answer: realpathSync(process.execPath) },
    { prompt: 'Allowed workspace (absolute directory): ', answer: dir },
    { prompt: 'Additional allowed workspace (blank for none): ', answer: workspace },
    { prompt: 'Absolute fixture TypeScript script: ', answer: resolve('test/runner.ts') },
    { prompt: 'Create a NEW agent identity assigned exclusively to this host? [yes/no]: ', answer: 'yes' },
    { prompt: 'Add another independent NEW agent using this same harness setup? [yes/no]: ', answer: 'yes' },
    { prompt: 'Add another independent NEW agent using this same harness setup? [yes/no]: ', answer: 'no' },
  ]);
  const manifest = readPrivate(join(source, 'setup.json')) as any;
  const [X, Y] = Object.keys(manifest.agents); assert.ok(X && Y); assert.notEqual(X, Y);
  assert.equal(Object.keys(manifest.setups).length, 1);
  assert.equal(manifest.agents[X].setup, manifest.agents[Y].setup);
  for (const key of [secret, manifest.agents[X].secret, manifest.agents[Y].secret]) assert.ok(!setupOutput.includes(key));
  const slots = installationSlots(source), first = slots.find(s => publicKey(s.setup.agentSecret) === X)!;
  const journal = (key: string) => JSON.parse(readFileSync(slots.find(s => publicKey(s.setup.agentSecret) === key)!.path, 'utf8'));
  for (const path of [join(source, 'setup.json'), ...slots.map(s => s.path)]) assert.equal(statSync(path).mode & 0o777, 0o600);
  assert.equal(statSync(source).mode & 0o777, 0o700);
  const manifestBefore = readFileSync(join(source, 'setup.json'));
  provision(target, { ...first.setup, host: 'target' }, journal(X).assignment.genesis);
  const server = await relay(0, publicKey(secret), join(dir, 'relay.json'));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`, seen: Message[] = [];
  const h = await host(source, url), t = await host(target, url), client = connect(url, secret, m => seen.push(m)); await client.ready;
  const targetJournal = () => JSON.parse(readFileSync(join(target, 'journal.json'), 'utf8'));
  async function request(type: Message['type'], hostName: string, revision: number, body = {}) {
    const m = message(type, hostName, X!, revision, body); client.send(m);
    await until(() => seen.some(r => r.type === 'receipt' && r.body.operation === m.id));
    return seen.find(r => r.type === 'receipt' && r.body.operation === m.id)!;
  }
  let actual: any, sibling: Buffer;
  try {
    const p = 'beehive> ';
    await terminal(['tui', identity, url], [
      { prompt: p, answer: 'hosts' }, { prompt: p, answer: 'select 1' },
      { prompt: p, answer: 'start' },
      { prompt: p, answer: 'select 2', gate: () => journal(X).phase === 'running' },
      { prompt: p, answer: 'start' },
      { prompt: p, answer: 'select 1', gate: () => { if (journal(Y).phase !== 'running') return false; actual = journal(X).actual; sibling = readFileSync(slots[1]!.path); return true; } },
      { prompt: p, answer: 'config-new' },
      { prompt: 'New configuration name (copies selected-next, same identity/setup): ', answer: 'Alternative' },
      { prompt: 'actual run unchanged? [yes/no]: ', answer: 'yes' },
      { prompt: p, answer: 'save', gate: () => journal(X).revision === 2 },
      { prompt: 'Model: ', answer: 'fixture-model' }, { prompt: 'Workspace: ', answer: workspace }, { prompt: 'Behavior profile: ', answer: 'default' },
      { prompt: p, answer: 'config-select default', gate: () => journal(X).revision === 3 },
      { prompt: 'actual run unchanged? [yes/no]: ', answer: 'yes' },
      { prompt: p, answer: 'config-select Alternative', gate: () => journal(X).revision === 4 },
      { prompt: 'actual run unchanged? [yes/no]: ', answer: 'yes' },
      { prompt: p, answer: 'configurations', gate: () => { if (journal(X).revision !== 5) return false; assert.deepEqual(journal(X).actual, actual); assert.deepEqual(readFileSync(slots[1]!.path), sibling); return true; } },
      { prompt: p, answer: 'restart' },
      { prompt: p, answer: 'show', gate: () => { if (journal(X).revision !== 6) return false; assert.equal(journal(X).actual.selection.configuration.name, 'Alternative'); assert.equal(journal(X).actual.selection.configuration.revision, 3); assert.equal(journal(X).actual.selection.workspace, workspace); assert.equal(journal(X).actual.harnessSetup.id, 'default'); assert.deepEqual(journal(X).actual.harnessSetup, journal(Y).actual.harnessSetup); assert.deepEqual(journal(X).runs[actual.run], actual); assert.deepEqual(readFileSync(slots[1]!.path), sibling); return true; } },
      { prompt: p, answer: 'select target' }, { prompt: p, answer: 'config-new' },
      { prompt: 'New configuration name (copies selected-next, same identity/setup): ', answer: 'Destination' },
      { prompt: 'actual run unchanged? [yes/no]: ', answer: 'yes' },
      { prompt: p, answer: 'save', gate: () => targetJournal().revision === 1 },
      { prompt: 'Model: ', answer: 'fixture-model' }, { prompt: 'Workspace: ', answer: workspace }, { prompt: 'Behavior profile: ', answer: 'default' },
      { prompt: p, answer: 'start', gate: () => targetJournal().revision === 2 },
      { prompt: p, answer: 'quit' },
    ]);
    await until(() => seen.some(m => m.type === 'receipt' && m.host === 'target' && m.body.result === 'not-authority'));
    assert.equal(targetJournal().phase, 'stopped'); assert.equal(targetJournal().assignment.assignedHost, 'source');
    const old = journal(X).actual;
    const stale = { ...targetJournal().selected, configuration: { name: 'Destination', revision: 1 } };
    assert.match(String((await request('move', 'source', 6, { target: 'target', targetRevision: 2, selection: stale })).body.result), /preflight failed/);
    assert.deepEqual(journal(X).actual, old);
    assert.equal((await request('move', 'source', 6, { target: 'target', targetRevision: 2, selection: targetJournal().selected })).body.result, 'accepted');
    await until(() => targetJournal().phase === 'running');
    assert.equal(targetJournal().actual.selection.configuration.name, 'Destination');
    assert.equal(targetJournal().actual.selection.workspace, workspace);
    assert.deepEqual(readFileSync(join(source, 'setup.json')), manifestBefore, 'remote configuration never rewrites/copies keys/setup');
    assert.deepEqual(readFileSync(slots[1]!.path), sibling!);
    const targetActual = targetJournal().actual, revision = targetJournal().revision;
    assert.match(String((await request('save', 'target', revision, { model: 'unsupported', workspace, profile: 'default' })).body.result), /Unsupported/);
    assert.equal(targetJournal().revision, revision); assert.deepEqual(targetJournal().actual, targetActual);
    assert.match(String((await request('save', 'target', revision, { configurationAction: 'remove', name: 'Destination' })).body.result), /Select another/);
    assert.equal((await request('save', 'target', revision, { configurationAction: 'rename', name: 'Destination', newName: 'Renamed' })).body.result, 'saved; running configuration unchanged');
    assert.deepEqual(targetJournal().actual, targetActual); assert.equal(targetJournal().selected.configuration.name, 'Renamed');
    assert.equal((await request('save', 'target', revision + 1, { configurationAction: 'remove', name: 'default' })).body.result, 'saved; running configuration unchanged');
    assert.equal(targetJournal().configurations.default, undefined); assert.deepEqual(targetJournal().actual, targetActual);
    console.log(`Wizard + named TUI: two identities ${X}, ${Y}; one setup, 0600 files/0700 directory, immutable old run, exact Restart Alternative@3, standby Destination@2 then Move; no key rewrite.`);
  } finally {
    await h.close(); await t.close(); client.close(); for (const c of server.clients) c.terminate();
    await new Promise<void>(r => server.close(() => r())); rmSync(dir, { recursive: true, force: true });
  }
});

test('named candidate transactions: safe rename/removal, bounds, no mutation and no revision reuse', () => {
  const selected = { model: 'fixture-model', workspace: '/tmp', profile: 'default' };
  const a = configure(undefined, selected, { configurationAction: 'create', name: 'A' }, 2);
  assert.throws(() => configure(a.entries, a.selected, { configurationAction: 'remove', name: 'A' }, 3), /Select another/);
  const b = configure(a.entries, a.selected, { configurationAction: 'rename', name: 'A', newName: 'B' }, 3);
  assert.equal(a.selected.configuration!.name, 'A'); assert.equal(b.selected.configuration!.revision, 3);
  assert.throws(() => configure(b.entries, b.selected, { configurationAction: 'create', name: '__proto__' }, 4), /Invalid/);
  assert.throws(() => configure(b.entries, b.selected, { configurationAction: 'create', name: 'B' }, 4), /exists/);
  const c = configure(b.entries, b.selected, { configurationAction: 'select', name: 'default' }, 4);
  const d = configure(c.entries, c.selected, { configurationAction: 'remove', name: 'B' }, 5);
  const e = configure(d.entries, d.selected, { configurationAction: 'create', name: 'B' }, 6);
  assert.equal(e.selected.configuration!.revision, 6);
});
