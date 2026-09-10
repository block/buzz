import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, type ChildProcess } from 'node:child_process';
import { once } from 'node:events';
import { mkdtempSync, realpathSync, rmSync, readFileSync, existsSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { setTimeout as delay } from 'node:timers/promises';
import { createGenesis, validateGenesis } from '../src/assignment.ts';
import { provision, host, type Setup } from '../src/host.ts';
import { migrateSlots, addSlot, removeSlotKey } from '../src/slots.ts';
import { newKey, publicKey, message, type Message } from '../src/protocol.ts';
import { readPrivate, writePrivate } from '../src/storage.ts';
import { relay } from '../src/relay.ts';
import { connect } from '../src/client.ts';

async function until(check: () => boolean) {
  for (let i = 0; i < 500; i++) { if (check()) return; await delay(20); }
  throw Error('Missing key-removal evidence');
}
async function closeChild(child: ChildProcess) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const done = once(child, 'exit'); child.kill('SIGTERM'); await done;
}
/** Actual CLI subprocess path; answers are matched against real prompts. */
async function cli(args: string[], answers: { prompt: string; answer: string }[] = [], timeoutMs = 20000): Promise<{ code: number | null; out: string; err: string }> {
  const child = spawn(process.execPath, ['src/cli.ts', ...args], { stdio: ['pipe', 'pipe', 'pipe'] });
  let out = '', err = '', cursor = 0, index = 0, pending = false;
  child.stderr.on('data', c => { err += c.toString(); });
  child.stdout.on('data', c => {
    out += c.toString(); const step = answers[index];
    if (!step || pending) return;
    const position = out.indexOf(step.prompt, cursor); if (position < 0) return;
    pending = true;
    void (async () => { await delay(60); cursor = position + step.prompt.length; index++; pending = false; child.stdin.write(`${step.answer}\n`); })();
  });
  const timer = setTimeout(() => child.kill('SIGTERM'), timeoutMs);
  try { const [code] = await once(child, 'exit'); return { code, out, err }; }
  finally { clearTimeout(timer); if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM'); }
}

test('deliberate local key removal keeps the public slot: CLI confirmation, retained sibling/history, reopen, denied Start/Restart before spawn, neutral Stop/Save, reconnect', { timeout: 40000 }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-remove-')));
  const ownerSecret = newKey(), x = newKey(), y = newKey(); const X = publicKey(x), Y = publicKey(y);
  const source = join(dir, 'source');
  const setup: Setup = { host: 'source', ownerSecret, agentSecret: x, runner: process.execPath, args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' };
  provision(source, setup, createGenesis(publicKey(ownerSecret), X, 'source'));
  migrateSlots(source); addSlot(source, y, createGenesis(publicKey(ownerSecret), Y, 'source'));
  const server = await relay(0, publicKey(ownerSecret), join(dir, 'relay.json'));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`; const seen: Message[] = [];
  const ui = connect(url, ownerSecret, m => seen.push(m)); await ui.ready;
  const journalPath = join(source, 'journal.json'), siblingPath = join(source, 'agents', Y, 'journal.json');
  const journal = () => JSON.parse(readFileSync(journalPath, 'utf8'));
  const manifest = () => JSON.parse(readFileSync(join(source, 'setup.json'), 'utf8'));
  async function request(m: Message) {
    ui.send(m); await until(() => seen.some(r => r.type === 'receipt' && r.body.operation === m.id && r.agent === m.agent));
    return seen.find(r => r.type === 'receipt' && r.body.operation === m.id && r.agent === m.agent)!;
  }
  let h: Awaited<ReturnType<typeof host>> | undefined;
  try {
    h = await host(source, url);
    assert.equal((await request(message('start', 'source', X, 0))).body.result, 'accepted');
    await until(() => journal().phase === 'running');
    assert.equal((await request(message('stop', 'source', X, 1))).body.result, 'accepted');
    await until(() => journal().phase === 'stopped');
    await h.close(); h = undefined;
    assert.equal(existsSync(join(source, 'host.lock')), false, 'clean close releases the installation lock');
    const xJournalBefore = readFileSync(journalPath), yJournalBefore = readFileSync(siblingPath), manifestBefore = readFileSync(join(source, 'setup.json'));
    // Confirmation is required: declining changes nothing.
    const declined = await cli(['remove-agent-key', source, X], [{ prompt: 'Remove this installation', answer: 'no' }]);
    assert.equal(declined.code, 0, declined.err);
    assert.deepEqual(readFileSync(join(source, 'setup.json')), manifestBefore, 'declined removal must not touch the manifest');
    // Multi-slot hosts require the exact agent public key; never a secret argument.
    for (const bad of await Promise.all([cli(['remove-agent-key', source]), cli(['remove-agent-key', source, 'not-hex'])])) {
      assert.equal(bad.code, 1); assert.match(bad.err, /Specify the agent public key|Invalid agent public key/);
    }
    const removed = await cli(['remove-agent-key', source, X], [{ prompt: 'Remove this installation', answer: 'yes' }]);
    assert.equal(removed.code, 0, removed.err);
    assert.match(removed.out, /Local key copy removed for agent/); assert.match(removed.out, /public-only slot retained/);
    assert.ok(!removed.out.includes(x) && !removed.err.includes(x), 'the removed secret is never printed');
    const after = manifest();
    assert.equal(after.agents[X].secret, null, 'only the local secret copy is deleted');
    assert.equal(after.agents[Y].secret, y, 'sibling key copy retained');
    assert.ok(!readFileSync(join(source, 'setup.json'), 'utf8').includes(x), 'secret bytes leave the 0600 manifest');
    assert.equal(statSync(join(source, 'setup.json')).mode & 0o777, 0o600);
    assert.equal(statSync(source).mode & 0o777, 0o700);
    assert.equal(statSync(join(source, 'agents')).mode & 0o777, 0o700);
    assert.equal(statSync(join(source, 'agents', Y)).mode & 0o777, 0o700);
    assert.equal(statSync(journalPath).mode & 0o777, 0o600);
    assert.deepEqual(readFileSync(journalPath), xJournalBefore, 'journal history is retained byte-identically');
    assert.deepEqual(readFileSync(siblingPath), yJournalBefore, 'sibling journal untouched');
    // Public genesis export still works for a public-only slot; it carries no secret.
    const exported = join(dir, 'genesis-export.json');
    const exportRun = await cli(['assignment-export', source, exported, X]);
    assert.equal(exportRun.code, 0, exportRun.err); assert.match(exportRun.out, /Public genesis exported; no keys/);
    assert.equal(validateGenesis(readPrivate(exported)).agent, X);
    assert.ok(!readFileSync(exported, 'utf8').includes(x));
    // A removed identity cannot be silently re-created; repeated removal is explicit.
    assert.throws(() => addSlot(source, x, createGenesis(publicKey(ownerSecret), X, 'source')), /Public-only slot retains this identity/);
    assert.throws(() => removeSlotKey(source, X), /already removed locally/);
    const again = await cli(['remove-agent-key', source, X], [{ prompt: 'Remove this installation', answer: 'yes' }]);
    assert.equal(again.code, 1); assert.match(again.err, /already removed locally/);
    // Reopen advertises the missing-key public slot with its retained identity.
    h = await host(source, url);
    await until(() => seen.some(m => m.type === 'inventory' && m.agent === X && m.body.localKey === 'removed locally; public slot retained'));
    const inventoryX = seen.find(m => m.type === 'inventory' && m.agent === X && m.body.localKey === 'removed locally; public slot retained')!;
    assert.match(String(inventoryX.body.readiness), /agent key removed locally: public-only slot/);
    assert.equal(inventoryX.body.assignedHost, 'source'); assert.equal(inventoryX.body.phase, 'stopped');
    await until(() => seen.some(m => m.type === 'inventory' && m.agent === Y && m.body.localKey === 'present'));
    // Start/Restart reject BEFORE any spawn; no secret is regenerated or inferred.
    const instructionLines = readFileSync(join(dir, 'received-instructions.jsonl'), 'utf8').trim().split('\n').length;
    const deniedStart = await request(message('start', 'source', X, journal().revision));
    assert.match(String(deniedStart.body.result), /Local agent key removed; this public slot cannot launch/);
    const deniedRestart = await request(message('restart', 'source', X, journal().revision));
    assert.match(String(deniedRestart.body.result), /Local agent key removed; this public slot cannot launch/);
    assert.equal(journal().phase, 'stopped'); assert.equal(journal().actual, null);
    assert.equal(readFileSync(join(dir, 'received-instructions.jsonl'), 'utf8').trim().split('\n').length, instructionLines, 'denied launches spawn nothing');
    assert.equal(manifest().agents[X].secret, null, 'no remote operation recreates the secret');
    // Stop/Save remain credential-neutral and truthful.
    assert.equal((await request(message('save', 'source', X, journal().revision, { model: 'fixture-model', workspace: dir, profile: 'default' }))).body.result, 'saved; running configuration unchanged');
    assert.equal((await request(message('stop', 'source', X, journal().revision))).body.result, 'accepted');
    assert.equal(journal().phase, 'stopped'); assert.ok(Object.keys(journal().runs).length >= 1, 'run history retained');
    // The untouched sibling still launches normally.
    assert.equal((await request(message('start', 'source', Y, 0))).body.result, 'accepted');
    await until(() => JSON.parse(readFileSync(siblingPath, 'utf8')).phase === 'running');
    // Management-socket loss replays receipts and the public-only inventory truthfully.
    seen.length = 0; const hostSocket = [...server.clients][1]!; hostSocket.terminate();
    await until(() => seen.some(m => m.type === 'inventory' && m.agent === X && m.body.localKey === 'removed locally; public slot retained'));
    await until(() => seen.some(m => m.type === 'receipt' && m.agent === X));
    await h.close(); h = undefined;
    assert.equal(manifest().agents[X].secret, null, 'ordinary lifecycle never recreates the removed key');
    console.log(`Public-only slot retained for ${X}; sibling ${Y} and history byte-identical; Start/Restart denied before spawn`);
  } finally {
    await h?.close(); ui.close(); for (const c of server.clients) c.terminate();
    await new Promise<void>(resolve => server.close(() => resolve())); rmSync(dir, { recursive: true, force: true });
  }
});

test('key removal refuses an active host lock and a non-stopped slot; legacy key-less installations fail closed', { timeout: 30000 }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-remove-fence-')));
  const ownerSecret = newKey(), x = newKey(); const X = publicKey(x);
  const source = join(dir, 'source');
  provision(source, { host: 'source', ownerSecret, agentSecret: x, runner: process.execPath, args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' }, createGenesis(publicKey(ownerSecret), X, 'source'));
  migrateSlots(source);
  const server = await relay(0, publicKey(ownerSecret), join(dir, 'relay.json'));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`; const seen: Message[] = [];
  const ui = connect(url, ownerSecret, m => seen.push(m)); await ui.ready;
  const journalPath = join(source, 'journal.json');
  const journal = () => JSON.parse(readFileSync(journalPath, 'utf8'));
  async function request(m: Message) {
    ui.send(m); await until(() => seen.some(r => r.type === 'receipt' && r.body.operation === m.id));
    return seen.find(r => r.type === 'receipt' && r.body.operation === m.id)!;
  }
  let h: Awaited<ReturnType<typeof host>> | undefined;
  try {
    h = await host(source, url);
    // A running host keeps its lock: refusal, never a PID or stale-lock deletion.
    const active = await cli(['remove-agent-key', source], [{ prompt: 'Remove this installation', answer: 'yes' }]);
    assert.equal(active.code, 1, active.out); assert.match(active.err, /Installation lock present/);
    assert.throws(() => removeSlotKey(source, X), /Installation lock present/);
    assert.equal(existsSync(join(source, 'host.lock')), true, 'refusal never removes a live lock');
    assert.equal((JSON.parse(readFileSync(join(source, 'setup.json'), 'utf8')) as { agents: Record<string, { secret: string }> }).agents[X].secret, x);
    assert.equal((await request(message('save', 'source', X, 0, { model: 'fixture-model', workspace: dir, profile: 'default' }))).body.result, 'saved; running configuration unchanged', 'host stays alive after refusal');
    await h.close(); h = undefined;
    assert.equal(existsSync(join(source, 'host.lock')), false);
    // Unknown or malformed identities and non-stopped slots fail closed before any edit.
    assert.throws(() => removeSlotKey(source, 'f'.repeat(64)), /Unknown agent slot/);
    assert.throws(() => removeSlotKey(source, 'zz'), /Invalid agent public key/);
    const liveLooking = journal(); writePrivate(journalPath, { ...liveLooking, phase: 'running' });
    assert.throws(() => removeSlotKey(source, X), /Key removal requires a stopped slot with no actual run/);
    writePrivate(journalPath, liveLooking);
    // The same clean state then removes deliberately (positive control).
    const removed = await cli(['remove-agent-key', source], [{ prompt: 'Remove this installation', answer: 'yes' }]);
    assert.equal(removed.code, 0, removed.err);
    assert.equal((JSON.parse(readFileSync(join(source, 'setup.json'), 'utf8')) as { agents: Record<string, { secret: string | null }> }).agents[X].secret, null);
    // A legacy (non-slots) installation whose local key copy is absent fails closed everywhere.
    const legacy = join(dir, 'legacy');
    provision(legacy, { host: 'legacy', ownerSecret, agentSecret: x, runner: process.execPath, args: [], workspace: dir, mode: 'buzz-agent-databricks-v2' }, createGenesis(publicKey(ownerSecret), X, 'legacy'));
    const stripped = readPrivate(join(legacy, 'setup.json')) as Record<string, unknown>; delete stripped.agentSecret;
    writePrivate(join(legacy, 'setup.json'), stripped);
    for (const args of [['remove-agent-key', legacy], ['migrate-assignment', legacy], ['conversation-setup', legacy]]) {
      const run = await cli(args); assert.equal(run.code, 1, run.out); assert.match(run.err, /Legacy setup requires its agent key/);
    }
    const migrateRun = await cli(['migrate-slots', legacy], [{ prompt: 'Upgrade this stopped installation', answer: 'yes' }]);
    assert.equal(migrateRun.code, 1, migrateRun.out); assert.match(migrateRun.err, /Legacy setup requires its agent key/);
    await assert.rejects(host(legacy, 'ws://127.0.0.1:9'), /Legacy setup requires its agent key/);
    assert.equal(existsSync(join(legacy, 'host.lock')), false, 'failed startup unwinds its own lock');
    // Migrated public-only slots cannot attach conversation setup: that would need the key.
    const databricks = join(dir, 'databricks');
    provision(databricks, { host: 'databricks', ownerSecret, agentSecret: x, runner: process.execPath, args: [], workspace: dir, mode: 'buzz-agent-databricks-v2' }, createGenesis(publicKey(ownerSecret), X, 'databricks'));
    migrateSlots(databricks); removeSlotKey(databricks, X);
    const conversation = await cli(['conversation-setup', databricks]);
    assert.equal(conversation.code, 1, conversation.out); assert.match(conversation.err, /conversation setup requires its agent key/);
  } finally {
    await h?.close(); ui.close(); for (const c of server.clients) c.terminate();
    await new Promise<void>(resolve => server.close(() => resolve())); rmSync(dir, { recursive: true, force: true });
  }
});

test('Move to a missing-key destination fails destination preflight and preserves the running source', { timeout: 40000 }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-remove-move-target-')));
  const ownerSecret = newKey(), x = newKey(); const X = publicKey(x);
  const genesis = createGenesis(publicKey(ownerSecret), X, 'source');
  const setup = (name: string): Setup => ({ host: name, ownerSecret, agentSecret: x, runner: process.execPath, args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' });
  const source = join(dir, 'source'), target = join(dir, 'target');
  provision(source, setup('source'), genesis); provision(target, setup('target'), genesis);
  migrateSlots(target); removeSlotKey(target, X);
  const targetManifestBefore = readFileSync(join(target, 'setup.json'));
  const server = await relay(0, publicKey(ownerSecret), join(dir, 'relay.json'));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`, seen: Message[] = [], children: ChildProcess[] = [];
  const ui = connect(url, ownerSecret, m => seen.push(m)); await ui.ready;
  const sourceJournal = () => JSON.parse(readFileSync(join(source, 'journal.json'), 'utf8'));
  const targetJournal = () => JSON.parse(readFileSync(join(target, 'journal.json'), 'utf8'));
  async function request(type: Message['type'], host: string, revision: number, body = {}) {
    const m = message(type, host, X, revision, body); ui.send(m);
    await until(() => seen.some(r => r.type === 'receipt' && r.body.operation === m.id && r.host === host));
    return seen.find(r => r.type === 'receipt' && r.body.operation === m.id && r.host === host)!;
  }
  try {
    for (const name of ['source', 'target']) {
      const child = spawn(process.execPath, ['src/cli.ts', 'host', join(dir, name), url], { stdio: ['ignore', 'pipe', 'pipe'] });
      children.push(child); child.stdout?.resume(); child.stderr?.resume();
      await until(() => seen.some(m => m.type === 'inventory' && m.host === name));
    }
    await until(() => seen.some(m => m.type === 'inventory' && m.host === 'target' && m.body.localKey === 'removed locally; public slot retained'));
    assert.equal((await request('start', 'source', 0)).body.result, 'accepted');
    await until(() => sourceJournal().phase === 'running');
    const sourceRun = sourceJournal().actual.run;
    const move = await request('move', 'source', sourceJournal().revision, { target: 'target', targetRevision: targetJournal().revision, selection: { model: 'fixture-model', workspace: dir, profile: 'default' } });
    assert.match(String(move.body.result), /Destination preflight failed/);
    assert.equal(sourceJournal().phase, 'running', 'source execution is preserved');
    assert.equal(sourceJournal().actual.run, sourceRun);
    assert.equal(sourceJournal().assignment.assignedHost, 'source', 'authority stays with the running source');
    assert.equal(targetJournal().phase, 'stopped'); assert.equal(targetJournal().actual, null);
    assert.equal(targetJournal().assignment.assignedHost, 'source');
    assert.deepEqual(readFileSync(join(target, 'setup.json')), targetManifestBefore, 'destination secret is not recreated by a Move attempt');
    // The failed reservation does not strand the source lifecycle.
    assert.equal((await request('stop', 'source', sourceJournal().revision)).body.result, 'accepted');
    await until(() => sourceJournal().phase === 'stopped');
  } finally {
    for (const child of children) await closeChild(child);
    ui.close(); for (const c of server.clients) c.terminate();
    await new Promise<void>(resolve => server.close(() => resolve())); rmSync(dir, { recursive: true, force: true });
  }
});

test('public-only source still consumes its assignment through Move; the destination launches with its own local key', { timeout: 40000 }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-remove-move-source-')));
  const ownerSecret = newKey(), x = newKey(); const X = publicKey(x);
  const genesis = createGenesis(publicKey(ownerSecret), X, 'a');
  const setup = (name: string): Setup => ({ host: name, ownerSecret, agentSecret: x, runner: process.execPath, args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' });
  const a = join(dir, 'a'), b = join(dir, 'b');
  provision(a, setup('a'), genesis); provision(b, setup('b'), genesis);
  migrateSlots(a); removeSlotKey(a, X);
  const aManifestBefore = readFileSync(join(a, 'setup.json'));
  const server = await relay(0, publicKey(ownerSecret), join(dir, 'relay.json'));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`, seen: Message[] = [], children: ChildProcess[] = [];
  const ui = connect(url, ownerSecret, m => seen.push(m)); await ui.ready;
  const journalOf = (name: string) => JSON.parse(readFileSync(join(dir, name, 'journal.json'), 'utf8'));
  try {
    for (const name of ['a', 'b']) {
      const child = spawn(process.execPath, ['src/cli.ts', 'host', join(dir, name), url], { stdio: ['ignore', 'pipe', 'pipe'] });
      children.push(child); child.stdout?.resume(); child.stderr?.resume();
      await until(() => seen.some(m => m.type === 'inventory' && m.host === name));
    }
    await until(() => seen.some(m => m.type === 'inventory' && m.host === 'a' && m.agent === X && m.body.localKey === 'removed locally; public slot retained'));
    const move = message('move', 'a', X, journalOf('a').revision, { target: 'b', targetRevision: journalOf('b').revision, selection: { model: 'fixture-model', workspace: dir, profile: 'default' } });
    ui.send(move);
    await until(() => seen.some(r => r.type === 'receipt' && r.body.operation === move.id && r.host === 'a'));
    const sourceReceipt = seen.find(r => r.type === 'receipt' && r.body.operation === move.id && r.host === 'a')!;
    assert.equal(sourceReceipt.body.result, 'accepted');
    assert.match(String(sourceReceipt.body.transfer), /source consumed/);
    await until(() => journalOf('b').phase === 'running' && journalOf('b').actual !== null);
    assert.equal(journalOf('b').assignment.assignedHost, 'b');
    assert.equal(journalOf('b').assignment.chain.length, 1);
    assert.equal(journalOf('a').assignment.assignedHost, 'b', 'public-only source consumed its retained management authority');
    assert.equal(journalOf('a').phase, 'stopped'); assert.equal(journalOf('a').actual, null);
    assert.deepEqual(readFileSync(join(a, 'setup.json')), aManifestBefore, 'Move never recreates or transfers the removed secret');
    assert.equal((JSON.parse(readFileSync(join(a, 'setup.json'), 'utf8')) as { agents: Record<string, { secret: string | null }> }).agents[X].secret, null);
    // The consumed source cannot Start again, from its own installation or elsewhere.
    const denied = message('start', 'a', X, journalOf('a').revision); ui.send(denied);
    await until(() => seen.some(r => r.type === 'receipt' && r.body.operation === denied.id));
    assert.equal(seen.find(r => r.type === 'receipt' && r.body.operation === denied.id)!.body.result, 'not-authority');
  } finally {
    for (const child of children) await closeChild(child);
    ui.close(); for (const c of server.clients) c.terminate();
    await new Promise<void>(resolve => server.close(() => resolve())); rmSync(dir, { recursive: true, force: true });
  }
});

test('K1 actual removal CLI validates retained host authority before destructive writes', { timeout: 30000 }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-k1-')));
  const ownerSecret = newKey(), x = newKey(), y = newKey(), X = publicKey(x), Y = publicKey(y);
  try {
    for (const mutation of ['missing-assignment', 'agent', 'owner', 'assigned-host', 'malformed-chain', 'root', 'predecessor', 'grant-agent', 'operation', 'stopped', 'standby', 'consumed-source']) {
      const d = join(dir, mutation);
      const genesis = createGenesis(publicKey(ownerSecret), X, 'source');
      provision(d, { host: 'source', ownerSecret, agentSecret: x, runner: process.execPath, args: [], workspace: dir, mode: 'fixture' }, genesis);
      migrateSlots(d); addSlot(d, y, createGenesis(publicKey(ownerSecret), Y, 'source'));
      const jp = join(d, 'journal.json'), mp = join(d, 'setup.json'), yp = join(d, 'agents', Y, 'journal.json');
      const state = JSON.parse(readFileSync(jp, 'utf8'));
      if (mutation === 'missing-assignment') delete state.assignment;
      if (mutation === 'agent') state.assignment.genesis.agent = Y;
      if (mutation === 'owner') state.assignment.genesis.owner = Y;
      if (mutation === 'assigned-host') state.assignment.assignedHost = 'other';
      if (mutation === 'malformed-chain') state.assignment.chain = {};
      if (mutation === 'standby') { state.assignment.genesis.initialHost = 'other'; state.assignment.assignedHost = 'other'; }
      if (['root', 'predecessor', 'grant-agent', 'operation', 'consumed-source'].includes(mutation)) {
        const { hash } = await import('../src/handoff.ts');
        const op = message('move', 'source', X, 0, { target: 'target', targetRevision: 0, selection: state.selected });
        const grant = { root: hash(genesis), predecessor: hash(genesis), source: 'source', target: 'target', agent: X, operation: op, prepared: 'fixture-token', selection: state.selected, targetRevision: 0, sourceRun: null };
        if (mutation === 'root') grant.root = 'wrong';
        if (mutation === 'predecessor') grant.predecessor = 'wrong';
        if (mutation === 'grant-agent') grant.agent = Y;
        if (mutation === 'operation') grant.operation = { ...op, host: 'wrong' };
        state.assignment.chain = [grant]; state.assignment.assignedHost = 'target';
      }
      writePrivate(jp, state);
      const journalBefore = readFileSync(jp), manifestBefore = readFileSync(mp), siblingBefore = readFileSync(yp);
      const run = await cli(['remove-agent-key', d, X], [{ prompt: 'Remove this installation', answer: 'yes' }]);
      const valid = ['stopped', 'standby', 'consumed-source'].includes(mutation);
      assert.equal(run.code, valid ? 0 : 1, `${mutation}: ${run.err}`);
      assert.equal(run.out.includes('Local key copy removed'), valid, mutation);
      assert.ok(!run.out.includes(x) && !run.err.includes(x));
      assert.deepEqual(readFileSync(jp), journalBefore, mutation);
      assert.deepEqual(readFileSync(yp), siblingBefore, mutation);
      assert.equal(existsSync(join(d, 'host.lock')), false);
      if (!valid) {
        assert.deepEqual(readFileSync(mp), manifestBefore, mutation);
        assert.match(run.err, /assignment|grant|chain|fields|message/i);
      } else {
        assert.equal(JSON.parse(readFileSync(mp, 'utf8')).agents[X].secret, null);
        const { loadSlotState } = await import('../src/host.ts');
        const { installationSlots } = await import('../src/slots.ts');
        const entry = installationSlots(d)[0]!;
        loadSlotState(entry.setup, entry.path, X);
      }
    }
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
