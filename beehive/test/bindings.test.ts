import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtempSync, realpathSync, mkdirSync, readFileSync, statSync, rmSync, rmdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { newKey, publicKey, message, type Message } from '../src/protocol.ts';
import { readPrivate, writePrivate } from '../src/storage.ts';
import { provision } from '../src/host.ts';
import { relay } from '../src/relay.ts';
import { connect } from '../src/client.ts';
import { installationSlots, addHarnessBinding, addSlot, migrateSlots, removeSlotKey } from '../src/slots.ts';
import { createGenesis } from '../src/assignment.ts';
import { hostReady } from './host-driver.ts';
import { provisionSetup } from './provision.ts';
import { profileRevision } from '../src/profiles.ts';

async function until(check: () => boolean) {
  for (let i = 0; i < 600; i++) { if (check()) return; await delay(20); }
  throw Error('Missing named-configuration evidence');
}
type Step = { prompt: string; answer: string; gate?: () => boolean; observedRevision?: number };
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
      if (step.observedRevision !== undefined) {
        // Journal commit is not proof the remote TUI has consumed inventory.
        const deadline = Date.now() + 8000;
        for (;;) {
          const from = output.length; child.stdin.write('show\n');
          await until(() => output.indexOf('beehive> ', from) >= 0);
          if (output.slice(from).includes(`\n  "revision": ${step.observedRevision},\n`)) break;
          assert.ok(Date.now() < deadline, 'TUI did not observe committed revision');
          await delay(20);
        }
      }
      await delay(80); cursor = output.length; index++; pending = false;
      child.stdin.write(`${step.answer}\n`);
    })().catch(e => { failure = e; child.kill('SIGTERM'); });
  });
  const timer = setTimeout(() => child.kill('SIGTERM'), 25000);
  try {
    const [code] = await once(child, 'exit');
    if (failure) throw new Error(`TUI step ${index} (${steps[index]?.answer}): ${String(failure)}\n${error}\n${output.slice(-24000)}`);
    assert.equal(code, 0, `${error}\n${output}`); assert.equal(index, steps.length, output);
    return output;
  } finally { clearTimeout(timer); if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM'); }
}

test('reusable A/B bindings: real remote named TUI and external Restart, stable identity/sibling/history; target-local Move', { timeout: 40000 }, async () => {
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
  const binding = { ...manifest.setups.default, workspace, allowedWorkspaces: [workspace], args: [resolve('test/binding-runner.ts')] };
  const localOutput = await terminal(['local-setup', source], [
    { prompt: 'Local action [', answer: 'add-binding' },
    { prompt: 'Existing binding ID to reuse: ', answer: 'default' },
    { prompt: 'NEW immutable binding ID: ', answer: 'B' },
    { prompt: 'Absolute compatible executable: ', answer: realpathSync(process.execPath) },
    { prompt: 'Allowed workspace (absolute directory): ', answer: workspace },
    { prompt: 'Absolute fixture TypeScript script: ', answer: resolve('test/binding-runner.ts') },
    { prompt: 'Save NEW binding only (no selection, key change or restart)? [yes/no]: ', answer: 'yes' },
  ]);
  assert.match(localOutput, /Binding B saved/);
  assert.deepEqual((readPrivate(join(source, 'setup.json')) as any).setups.B, binding);
  for (const key of [secret, manifest.agents[X].secret, manifest.agents[Y].secret]) assert.ok(!localOutput.includes(key));
  const slots = installationSlots(source), first = slots.find(s => s.agent === X)!;
  const journal = (key: string) => JSON.parse(readFileSync(slots.find(s => s.agent === key)!.path, 'utf8'));
  for (const path of [join(source, 'setup.json'), ...slots.map(s => s.path)]) assert.equal(statSync(path).mode & 0o777, 0o600);
  assert.equal(statSync(source).mode & 0o777, 0o700);
  const manifestBefore = readFileSync(join(source, 'setup.json'));
  provision(target, { ...first.setup, host: 'target' }, journal(X).assignment.genesis);
  migrateSlots(target); addHarnessBinding(target, 'B', { ...binding, args: [...binding.args, '--target-local'] });
  const server = await relay(0, publicKey(secret), join(dir, 'relay.json'));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`, seen: Message[] = [];
  async function launch(directory: string) {
    const child = spawn(process.execPath, ['src/cli.ts', 'host', directory, url], { stdio: ['ignore', 'pipe', 'pipe'] });
    await hostReady(child, directory);
    return { async close() { const exited = once(child, 'exit'); child.kill('SIGTERM'); await exited; } };
  }
  const h = await launch(source), t = await launch(target), client = connect(url, secret, m => seen.push(m)); await client.ready;
  const instructions = 'B selected profile marker';
  const behavior = { name: 'Binding behavior', parent: null, instructions, revision: profileRevision('Binding behavior', null, instructions) };
  const publication = message('profile', 'profiles', 'profiles', 0, behavior); client.send(publication);
  await until(() => seen.some(m => m.id === publication.id));
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
      { prompt: p, answer: 'config-new', observedRevision: 1 },
      { prompt: 'New configuration name (copies selected-next, same identity/setup): ', answer: 'Alternative' },
      { prompt: 'actual run unchanged? [yes/no]: ', answer: 'yes' },
      { prompt: p, answer: 'binding B', observedRevision: 2, gate: () => journal(X).revision === 2 },
      { prompt: p, answer: 'profiles', observedRevision: 3, gate: () => journal(X).revision === 3 },
      { prompt: p, answer: 'apply 1', observedRevision: 3 },
      { prompt: p, answer: 'config-select default', observedRevision: 4, gate: () => journal(X).revision === 4 },
      { prompt: 'actual run unchanged? [yes/no]: ', answer: 'yes' },
      { prompt: p, answer: 'config-select Alternative', observedRevision: 5, gate: () => journal(X).revision === 5 },
      { prompt: 'actual run unchanged? [yes/no]: ', answer: 'yes' },
      { prompt: p, answer: 'configurations', gate: () => { if (journal(X).revision !== 6) return false; assert.deepEqual(journal(X).actual, actual); assert.deepEqual(readFileSync(slots[1]!.path), sibling); return true; } },
      { prompt: p, answer: 'restart', observedRevision: 6 },
      { prompt: p, answer: 'show', gate: () => { if (journal(X).revision !== 7) return false; assert.equal(journal(X).actual.selection.configuration.name, 'Alternative'); assert.equal(journal(X).actual.selection.configuration.revision, 4); assert.equal(journal(X).actual.selection.workspace, workspace); assert.equal(journal(X).actual.harnessSetup.id, 'B'); assert.notDeepEqual(journal(X).actual.harnessSetup, journal(Y).actual.harnessSetup); assert.equal(journal(Y).actual.harnessSetup.id, 'default'); assert.deepEqual(journal(X).actual.selection.harnessSetup, journal(X).actual.harnessSetup); assert.deepEqual(journal(X).runs[actual.run], actual); assert.deepEqual(readFileSync(slots[1]!.path), sibling); return true; } },
      { prompt: p, answer: 'select target' }, { prompt: p, answer: 'config-new' },
      { prompt: 'New configuration name (copies selected-next, same identity/setup): ', answer: 'Destination' },
      { prompt: 'actual run unchanged? [yes/no]: ', answer: 'yes' },
      { prompt: p, answer: 'binding B', observedRevision: 1, gate: () => targetJournal().revision === 1 },
      { prompt: p, answer: 'start', observedRevision: 2, gate: () => targetJournal().revision === 2 },
      { prompt: p, answer: 'quit' },
    ]);
    await until(() => seen.some(m => m.type === 'receipt' && m.host === 'target' && m.body.result === 'not-authority'));
    assert.equal(targetJournal().phase, 'stopped'); assert.equal(targetJournal().assignment.assignedHost, 'source');
    await until(() => { try { return readFileSync(join(workspace, 'binding-B.jsonl'), 'utf8').includes('B'); } catch { return false; } });
    const marker = JSON.parse(readFileSync(join(workspace, 'binding-B.jsonl'), 'utf8').trim().split('\n')[0]!);
    assert.equal(marker.instructions, instructions);
    assert.equal(journal(X).actual.appliedInstructions.revision, behavior.revision);
    assert.notEqual(journal(X).actual.run, actual.run);
    assert.equal(journal(X).binding.agent, X);
    assert.throws(() => addHarnessBinding(source, 'locked', binding), /EEXIST/);
    assert.deepEqual(readFileSync(join(source, 'setup.json')), manifestBefore);
    const replay = seen.find(m => m.type === 'save' && m.host === 'source' && (m.body.harnessSetup as any)?.id === 'B')!;
    assert.ok(replay);
    const beforeReplay = readFileSync(first.path); client.send(replay); await delay(100);
    assert.deepEqual(readFileSync(first.path), beforeReplay);
    assert.deepEqual(readFileSync(join(source, 'setup.json')), manifestBefore);
    const old = journal(X).actual;
    for (const harnessSetup of [{ id: 'unknown', fingerprint: old.harnessSetup.fingerprint }, { id: 'B', fingerprint: '0'.repeat(64) }]) {
      assert.notEqual((await request('save', 'source', 7, { ...journal(X).selected, harnessSetup })).body.result, 'saved; running configuration unchanged');
      assert.deepEqual(journal(X).actual, old);
    }
    const stale = { ...targetJournal().selected, profile: behavior.revision, behavior, configuration: { name: 'Destination', revision: 1 } };
    assert.match(String((await request('move', 'source', 7, { target: 'target', targetRevision: 2, selection: stale })).body.result), /preflight failed/);
    assert.deepEqual(journal(X).actual, old);
    assert.equal((await request('move', 'source', 7, { target: 'target', targetRevision: 2, selection: { ...targetJournal().selected, profile: behavior.revision, behavior } })).body.result, 'accepted');
    await until(() => targetJournal().phase === 'running');
    assert.equal(targetJournal().actual.selection.configuration.name, 'Destination');
    assert.equal(targetJournal().actual.selection.workspace, workspace);
    assert.equal(targetJournal().actual.harnessSetup.id, 'B');
    assert.equal(targetJournal().actual.appliedInstructions.revision, behavior.revision);
    assert.notEqual(targetJournal().actual.harnessSetup.fingerprint, old.harnessSetup.fingerprint);
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
    console.log(`Binding fingerprints: A=${actual.harnessSetup.fingerprint}; source B=${old.harnessSetup.fingerprint}; target B=${targetActual.harnessSetup.fingerprint}; profile=${behavior.revision}`);
    console.log(`A/B external runner + named TUI: two identities ${X}, ${Y}; two bindings, actual B profile marker, 0600 files/0700 directory, immutable old run, exact Restart Alternative@4, standby Destination@2 then Move; no key rewrite.`);
  } finally {
    await h.close(); await t.close(); client.close(); for (const c of server.clients) c.terminate();
    await new Promise<void>(r => server.close(() => r())); rmSync(dir, { recursive: true, force: true });
  }
});



test('local binding API shares atomic host.lock with competing service start; identity and private manifest retained', async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-binding-lock-')));
  try {
    provisionSetup(join(dir, 'setup.json'), { host: 'lock-fixture', ownerSecret: newKey(), agentSecret: newKey(), runner: realpathSync(process.execPath), args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' });
    migrateSlots(dir);
    const { host: _host, ownerSecret: _owner, agentSecret: _agent, ...binding } = installationSlots(dir)[0]!.setup;
    const before = readFileSync(join(dir, 'setup.json')), journal = readFileSync(join(dir, 'journal.json'));
    const lock = join(dir, 'host.lock'); mkdirSync(lock, { mode: 0o700 });
    try {
      assert.equal(statSync(lock).mode & 0o777, 0o700);
      assert.throws(() => addHarnessBinding(dir, 'B', binding), /EEXIST/);
      const competing = spawn(process.execPath, ['src/cli.ts', 'host', dir, 'ws://127.0.0.1:1'], { stdio: ['ignore', 'pipe', 'pipe'] });
      await assert.rejects(hostReady(competing, 'competing start during local mutation lock'), /EEXIST/);
      assert.equal(statSync(lock).mode & 0o777, 0o700, 'refused callers do not remove another owner lock');
      assert.deepEqual(readFileSync(join(dir, 'setup.json')), before);
    } finally { rmdirSync(lock); }
    assert.throws(() => addHarnessBinding(dir, 'B', { ...binding, ownerSecret: newKey() } as any), /identity/);
    assert.throws(() => addHarnessBinding(dir, 'B', { ...binding, conversation: {} } as any), /conversation authority/);
    assert.throws(() => addHarnessBinding(dir, 'B', binding, { id: 'default', fingerprint: '0'.repeat(64) }), /definition changed/);
    const fresh = newKey();
    assert.throws(() => addSlot(dir, fresh, createGenesis(publicKey(_owner), publicKey(fresh), _host), 'default', '0'.repeat(64)), /definition changed/);
    assert.deepEqual(readFileSync(join(dir, 'setup.json')), before, 'stale wizard inputs do not write');
    addHarnessBinding(dir, 'B', binding);
    assert.throws(() => addHarnessBinding(dir, 'B', binding), /exists/);
    const after = readPrivate(join(dir, 'setup.json')) as any, old = JSON.parse(before.toString());
    assert.deepEqual(after.agents, old.agents); assert.equal(after.ownerSecret, old.ownerSecret); assert.equal(after.host, old.host);
    assert.deepEqual(readFileSync(join(dir, 'journal.json')), journal);
    assert.equal(statSync(join(dir, 'setup.json')).mode & 0o777, 0o600);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test('local wizard reuses an identity without mutation, creates independent key on chosen binding, restores exact retained key hidden', { timeout: 15000 }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-local-wizard-')));
  const secret = newKey(), key = newKey();
  try {
    provisionSetup(join(dir, 'setup.json'), { host: 'local', ownerSecret: secret, agentSecret: key, runner: realpathSync(process.execPath), args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' });
    migrateSlots(dir);
    const original = installationSlots(dir)[0]!;
    const { host: _host, ownerSecret: _owner, agentSecret: _agent, ...binding } = original.setup;
    addHarnessBinding(dir, 'B', binding);
    const before = readFileSync(join(dir, 'setup.json')), journal = readFileSync(original.path);
    const reuse = await terminal(['local-setup', dir], [
      { prompt: 'Local action [', answer: 'reuse' },
      { prompt: 'Existing agent public key: ', answer: original.agent },
    ]);
    assert.match(reuse, /no local mutation/); assert.deepEqual(readFileSync(join(dir, 'setup.json')), before);
    const added = await terminal(['local-setup', dir], [
      { prompt: 'Local action [', answer: 'new-agent' },
      { prompt: 'Existing binding ID to reuse: ', answer: 'B' },
      { prompt: 'Create NEW independent identity using B? [yes/no]: ', answer: 'yes' },
    ]);
    const entries = installationSlots(dir); assert.equal(entries.length, 2);
    const second = entries.find(e => e.agent !== original.agent)!;
    assert.equal(second.setupId, 'B'); assert.notEqual(second.setup.agentSecret, key);
    assert.deepEqual(readFileSync(original.path), journal);
    const sibling = readFileSync(second.path);
    removeSlotKey(dir, original.agent);
    const restored = await terminal(['local-setup', dir], [
      { prompt: 'Local action [', answer: 'restore-key' },
      { prompt: 'Existing agent public key: ', answer: original.agent },
      { prompt: 'Restore exact retained key only, preserving assignment/history? [yes/no]: ', answer: 'yes' },
      { prompt: 'Agent private key (64 hex; hidden, never passed as an argument): ', answer: key },
    ]);
    assert.equal(installationSlots(dir)[0]!.setup.agentSecret, key);
    assert.deepEqual(readFileSync(original.path), journal); assert.deepEqual(readFileSync(second.path), sibling);
    for (const output of [reuse, added, restored]) for (const value of [secret, key, second.setup.agentSecret!]) assert.ok(!output.includes(value));
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
