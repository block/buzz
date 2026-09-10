import { migrateSlots, retireHarnessBinding, addHarnessBinding, installationSlots } from '../src/slots.ts';
import { bindingFingerprint } from '../src/host.ts';
import { profileRevision } from '../src/profiles.ts';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, realpathSync, rmSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { setTimeout as delay } from 'node:timers/promises';
import { createGenesis } from '../src/assignment.ts';
import { provision, host } from '../src/host.ts';
import { newKey, publicKey, message, open, type Message } from '../src/protocol.ts';
import { writePrivate } from '../src/storage.ts';
import { relay } from '../src/relay.ts';
import { connect } from '../src/client.ts';

for (const failure of ['changed-across-target-restart', 'retired-across-target-restart', 'remote-candidate-save'] as const) test(`explicit B binding profiled Move reserved revision; post-grant ${failure} leaves target assigned stopped`, { timeout: 20000 }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-move-loss-')));
  const ownerSecret = newKey(), agentSecret = newKey(), agent = publicKey(agentSecret);
  const genesis = createGenesis(publicKey(ownerSecret), agent, 'source');
  for (const name of ['source','target']) provision(join(dir,name), { host: name, ownerSecret, agentSecret, runner: process.execPath, args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' },genesis);
  for (const name of ['source', 'target']) {
    const directory = join(dir, name); migrateSlots(directory);
    const { host: _host, ownerSecret: _owner, agentSecret: _agent, ...binding } = installationSlots(directory)[0]!.setup;
    addHarnessBinding(directory, 'B', { ...binding, args: [resolve('test/binding-runner.ts'), name] });
  }
  const targetBinding = installationSlots(join(dir, 'target'))[0]!.bindings.B!;
  const ref = { id: 'B', fingerprint: bindingFingerprint(targetBinding) };
  const server = await relay(0, publicKey(ownerSecret), join(dir,'relay.json'));
  const address = server.address(); assert.ok(address && typeof address !== 'string'); const url = `ws://127.0.0.1:${address.port}`;
  let source = await host(join(dir,'source'),url); let target = await host(join(dir,'target'),url);
  const sourceSocket = [...server.clients][0]!;
  let lostOperation = '', loseReceipt = true;
  const handlers = sourceSocket.listeners('message'); sourceSocket.removeAllListeners('message');
  sourceSocket.on('message', (raw, binary) => {
    const m = open(JSON.parse(String(raw)),ownerSecret);
    if (loseReceipt && m.type === 'receipt' && m.body.operation === lostOperation) return;
    for (const handler of handlers) Reflect.apply(handler, sourceSocket, [raw,binary]);
  });
  const targetSocket = [...server.clients][1]!; const send = targetSocket.send.bind(targetSocket); let drop = true;
  targetSocket.send = ((data: Parameters<typeof send>[0], ...args: unknown[]) => {
    const m = open(JSON.parse(String(data)),ownerSecret);
    if (drop && m.type === 'grant') return;
    return (send as Function)(data,...args);
  }) as typeof targetSocket.send;
  const seen: Message[] = []; const ui = connect(url,ownerSecret,m => seen.push(m)); await ui.ready;
  const journal = (name: string) => JSON.parse(readFileSync(join(dir,name,'journal.json'),'utf8'));
  async function until(predicate: () => boolean) { for (let i=0;i<500;i++) { if (predicate()) return; await delay(10); } throw Error('Missing failure/recovery evidence'); }
  try {
    const behavior = { name: 'Source', parent: null, instructions: 'Reserved named source behavior', revision: profileRevision('Source', null, 'Reserved named source behavior') };
    ui.send(message('profile','profiles','profiles',0,behavior));
    await until(() => seen.some(m => m.type === 'profile'));
    ui.send(message('save','source',agent,0,{ ...journal('source').selected, profile: behavior.revision, behavior }));
    ui.send(message('save','target',agent,0,{ ...journal('target').selected, harnessSetup: ref }));
    await until(() => journal('source').revision === 1 && journal('target').revision === 1);
    const oldCandidate = journal('target').selected;
    const start = message('start','source',agent,1); ui.send(start); await until(() => journal('source').phase === 'running');
    const move = message('move','source',agent,2,{ target:'target',targetRevision:1,selection:{ ...oldCandidate, profile: behavior.revision, behavior } }); lostOperation = move.id; ui.send(move);
    await until(() => journal('source').assignment.assignedHost === 'target');
    assert.equal(journal('source').phase,'stopped'); assert.equal(journal('target').assignment.assignedHost,'source');
    assert.equal(seen.some(m => m.type === 'receipt' && m.body.operation === move.id),false, 'source receipt lost before relay storage');
    const originalPreparation = journal('target').preparations[move.id];
    assert.deepEqual(originalPreparation.candidate, oldCandidate);
    assert.equal(originalPreparation.reservedRevision, 2);
    const consumed = journal('source').assignment.chain.at(-1);
    assert.equal(consumed.selection.configuration.revision, 2);
    assert.equal(journal('target').revision, 2);
    // A stale pre-reservation Save must not consume/redefine the reserved revision.
    const stale = message('save','target',agent,1,oldCandidate); ui.send(stale);
    await until(() => seen.some(m => m.body.operation === stale.id && m.body.result === 'revision-conflict'));
    if (failure !== 'remote-candidate-save') await target.close();
    const targetSetup = join(dir,'target','setup.json');
    if (failure === 'remote-candidate-save') {
      const edit = message('save', 'target', agent, 2, oldCandidate); ui.send(edit);
      await until(() => journal('target').revision === 3);
      assert.equal(journal('target').selected.configuration.revision, 3); assert.equal(journal('target').selected.profile, 'default');
    } else if (failure === 'retired-across-target-restart') {
      const before = JSON.parse(readFileSync(targetSetup, 'utf8'));
      retireHarnessBinding(join(dir, 'target'), ref);
      assert.deepEqual(JSON.parse(readFileSync(targetSetup, 'utf8')).setups, before.setups);
    } else {
      // Fault injection: offline definition changed after consumed preparation.
      const manifest = JSON.parse(readFileSync(targetSetup, 'utf8'));
      manifest.setups.B.args.push('changed-after-preparation'); writePrivate(targetSetup, manifest);
    }
    if (failure !== 'remote-candidate-save') target = await host(join(dir,'target'),url);
    drop = false; loseReceipt = false;
    ui.send(message('inspect','source',agent));
    await until(() => seen.some(m => m.type === 'receipt' && m.body.operation === move.id && m.body.result === 'accepted'));
    await until(() => journal('target').assignment.assignedHost === 'target');
    await until(() => seen.some(m => m.host === 'target' && m.type === 'receipt' && String(m.body.result).includes(failure === 'remote-candidate-save' ? 'Destination candidate changed after preparation' : 'Destination preparation invalidated')));
    assert.equal(journal('target').phase,'stopped'); assert.equal(journal('target').actual,null);
    if (failure === 'remote-candidate-save') { assert.equal(journal('target').selected.configuration.revision, 3); assert.equal(journal('target').selected.profile, 'default'); }
    else { assert.deepEqual(journal('target').configurations.default, consumed.selection); assert.equal(journal('target').selected.behavior.instructions, behavior.instructions); }
    if (failure === 'changed-across-target-restart') {
      const staleStart = message('start', 'target', agent, journal('target').revision);
      ui.send(staleStart); await until(() => seen.some(m => m.body.operation === staleStart.id && m.body.result === 'Binding definition changed'));
      assert.equal(journal('target').actual, null);
    }
    if (failure === 'retired-across-target-restart') {
      for (const action of ['start', 'restart'] as const) {
        const denied = message(action, 'target', agent, journal('target').revision); ui.send(denied);
        await until(() => seen.some(m => m.body.operation === denied.id && String(m.body.result).includes('Binding retired')));
        assert.equal(journal('target').actual, null);
      }
    }
    const denied = message('start','source',agent,3); ui.send(denied);
    await until(() => seen.some(m => m.body.operation === denied.id && m.body.result === 'not-authority'));
    assert.deepEqual(journal('target').preparations[move.id], originalPreparation, 'replayed prepare never replaces consumed grant evidence');
    const state = journal('target');
    for (const grant of seen.filter(m => m.type === 'grant')) ui.send(grant);
    await delay(100); assert.deepEqual(journal('target'),state);
    await source.close(); source = await host(join(dir,'source'),url);
    const restarted = message('start','source',agent,3); ui.send(restarted);
    await until(() => seen.some(m => m.body.operation === restarted.id && m.body.result === 'not-authority'));
    assert.equal(journal('source').assignment.assignedHost,'target', 'service restart cannot resurrect consumed source');
  } finally {
    await source.close(); await target.close(); ui.close(); for (const c of server.clients) c.terminate();
    await new Promise<void>(resolve => server.close(() => resolve())); rmSync(dir,{ recursive:true,force:true });
  }
});
