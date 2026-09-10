import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, type ChildProcess } from 'node:child_process';
import { once } from 'node:events';
import { mkdtempSync, realpathSync, rmSync, readFileSync, unlinkSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { setTimeout as delay } from 'node:timers/promises';
import { createGenesis } from '../src/assignment.ts';
import { provision, host, type Setup } from '../src/host.ts';
import { migrateSlots, addSlot } from '../src/slots.ts';
import { newKey, publicKey, message, open, type Message } from '../src/protocol.ts';
import { writePrivate } from '../src/storage.ts';
import { relay } from '../src/relay.ts';
import { connect } from '../src/client.ts';

test('two real hosts: source-consumed Move, sibling isolation, preflight refusal, historical replay', { timeout: 30000 }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-move-')));
  const ownerSecret = newKey(), x = newKey(), y = newKey(), agent = publicKey(x);
  const genesis = createGenesis(publicKey(ownerSecret), agent, 'source');
  const setup = (host: string): Setup => ({ host, ownerSecret, agentSecret: x, runner: process.execPath, args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' });
  provision(join(dir, 'source'), setup('source'), genesis); provision(join(dir, 'target'), setup('target'), genesis);
  migrateSlots(join(dir, 'source')); addSlot(join(dir, 'source'), y, createGenesis(publicKey(ownerSecret), publicKey(y), 'source'));
  const server = await relay(0, publicKey(ownerSecret), join(dir, 'relay.json'));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`, seen: Message[] = [], children: ChildProcess[] = [];
  const ui = connect(url, ownerSecret, m => seen.push(m)); await ui.ready;
  async function wait(predicate: (m: Message) => boolean) {
    for (let i = 0; i < 500; i++) { const m = seen.find(predicate); if (m) return m; await delay(10); }
    throw Error('Missing Move evidence');
  }
  const journal = (name: string) => JSON.parse(readFileSync(join(dir, name, 'journal.json'), 'utf8'));
  async function request(type: Message['type'], host: string, revision: number, body = {}, key = agent) {
    const m = message(type, host, key, revision, body); ui.send(m);
    return wait(r => r.type === 'receipt' && r.body.operation === m.id && r.host === host);
  }
  try {
    for (const name of ['source','target']) {
      const child = spawn(process.execPath, ['src/cli.ts','host',join(dir,name),url], { stdio: ['ignore','pipe','pipe'] });
      children.push(child); child.stdout?.resume(); child.stderr?.resume(); await wait(m => m.type === 'inventory' && m.host === name);
    }
    assert.equal((await request('start','source',0)).body.result,'accepted');
    assert.equal((await request('start','source',0,{},publicKey(y))).body.result,'accepted');
    const siblingPath = join(dir,'source','agents',publicKey(y),'journal.json');
    const sibling = readFileSync(siblingPath,'utf8');
    assert.equal((await request('start','target',0)).body.result,'not-authority');
    const selected = { model: 'fixture-model', workspace: dir, profile: 'default' };
    const failed = await request('move','source',1,{ target: 'target', targetRevision: 9, selection: selected });
    assert.match(String(failed.body.result),/preflight failed/);
    assert.equal(journal('source').phase,'running'); assert.equal(journal('source').revision,1);
    writePrivate(join(dir,'identity.json'),{ secret: ownerSecret });
    const terminal = spawn(process.execPath,['src/cli.ts','tui',join(dir,'identity.json'),url],{ stdio:['pipe','pipe','pipe'] }); children.push(terminal);
    let output = '', cursor = 0, index = 0;
    const steps = [
      { prompt:'beehive> ', value:() => 'hosts' },
      { prompt:'beehive> ', value:() => { const row = output.match(new RegExp(`(\\d+)\\. source \\| agent ${agent}`)); assert.ok(row); return `select ${row[1]}`; } },
      { prompt:'beehive> ', value:() => 'move' },
      { prompt:'Destination number: ', value:() => '1' },
      { prompt:'Confirm [yes/no]: ', value:() => 'yes' },
      { prompt:'beehive> ', value:() => 'quit' },
    ];
    terminal.stdout.on('data',chunk => {
      output += chunk.toString(); const step = steps[index];
      if (!step) return;
      const position = output.indexOf(step.prompt, cursor); if (position < 0) return;
      index++; cursor = position + step.prompt.length;
      setImmediate(() => terminal.stdin.write(`${step.value()}\n`));
    }); terminal.stderr.resume();
    const [code] = await once(terminal,'exit'); assert.equal(code,0); assert.match(output,/Durably pending move/);
    const move = await wait(m => m.type === 'move' && m.host === 'source' && m.body.targetRevision === 0);

    const result = await wait(m => m.type === 'receipt' && m.body.operation === move.id && m.host === 'source'); assert.equal(result.body.result,'accepted');
    await wait(m => m.type === 'inventory' && m.host === 'target' && m.body.phase === 'running');
    assert.equal(journal('source').assignment.assignedHost,'target'); assert.equal(journal('source').phase,'stopped'); assert.equal(journal('source').actual,null);
    assert.equal(journal('target').assignment.assignedHost,'target'); assert.equal(journal('target').actual.run,move.id);
    assert.equal(readFileSync(siblingPath,'utf8'),sibling);
    const grant = await wait(m => m.type === 'grant'); const before = readFileSync(join(dir,'target','journal.json'),'utf8');
    ui.send(move); ui.send(grant); await delay(100); assert.equal(readFileSync(join(dir,'target','journal.json'),'utf8'),before);
    assert.equal((await request('start','source',2)).body.result,'not-authority');
    // Move back is a NEW successor; historical forward grant cannot seize it.
    assert.equal((await request('move','target',1,{ target: 'source', targetRevision: 2, selection: selected })).body.result,'accepted');
    await wait(m => m.type === 'inventory' && m.host === 'source' && m.revision === 3 && m.body.phase === 'running');
    ui.send(grant); await delay(100);
    assert.equal(journal('target').assignment.assignedHost,'source'); assert.equal(journal('target').phase,'stopped');
    assert.equal(journal('source').assignment.chain.length,2);
    assert.equal((await request('stop','source',3)).body.result,'accepted');
    assert.equal((await request('stop','source',1,{},publicKey(y))).body.result,'accepted');
  } finally {
    for (const child of children) { if (child.exitCode === null && child.signalCode === null) { const exit = once(child,'exit'); child.kill('SIGTERM'); await exit; } }
    ui.close(); for (const c of server.clients) c.terminate(); await new Promise<void>(resolve => server.close(() => resolve())); rmSync(dir,{ recursive:true,force:true });
  }
});


for (const failure of ['missing-key', 'changed-prepared-input', 'changed-across-target-restart'] as const) test(`dropped grant/receipt recovery; post-grant ${failure} leaves target assigned stopped`, { timeout: 20000 }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-move-loss-')));
  const ownerSecret = newKey(), agentSecret = newKey(), agent = publicKey(agentSecret);
  const genesis = createGenesis(publicKey(ownerSecret), agent, 'source');
  for (const name of ['source','target']) provision(join(dir,name), { host: name, ownerSecret, agentSecret, runner: process.execPath, args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' },genesis);
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
    const start = message('start','source',agent,0); ui.send(start); await until(() => journal('source').phase === 'running');
    const move = message('move','source',agent,1,{ target:'target',targetRevision:0,selection:{ model:'fixture-model',workspace:dir,profile:'default' } }); lostOperation = move.id; ui.send(move);
    await until(() => journal('source').assignment.assignedHost === 'target');
    assert.equal(journal('source').phase,'stopped'); assert.equal(journal('target').assignment.assignedHost,'source');
    assert.equal(seen.some(m => m.type === 'receipt' && m.body.operation === move.id),false, 'source receipt lost before relay storage');
    const originalPreparation = journal('target').preparations[move.id];
    if (failure === 'changed-across-target-restart') await target.close();
    const targetSetup = join(dir,'target','setup.json');
    if (failure === 'missing-key') unlinkSync(targetSetup);
    else writePrivate(targetSetup, { ...JSON.parse(readFileSync(targetSetup,'utf8')), args: [resolve('test/runner.ts'), 'changed-after-preparation'] });
    if (failure === 'changed-across-target-restart') target = await host(join(dir,'target'),url);
    drop = false; loseReceipt = false;
    ui.send(message('inspect','source',agent));
    await until(() => seen.some(m => m.type === 'receipt' && m.body.operation === move.id && m.body.result === 'accepted'));
    await until(() => journal('target').assignment.assignedHost === 'target');
    await until(() => seen.some(m => m.host === 'target' && m.type === 'receipt' && String(m.body.result).includes(failure === 'changed-across-target-restart' ? 'Destination preparation invalidated' : 'Local agent key/setup missing')));
    assert.equal(journal('target').phase,'stopped'); assert.equal(journal('target').actual,null);
    const denied = message('start','source',agent,2); ui.send(denied);
    await until(() => seen.some(m => m.body.operation === denied.id && m.body.result === 'not-authority'));
    assert.deepEqual(journal('target').preparations[move.id], originalPreparation, 'replayed prepare never replaces consumed grant evidence');
    const state = journal('target');
    for (const grant of seen.filter(m => m.type === 'grant')) ui.send(grant);
    await delay(100); assert.deepEqual(journal('target'),state);
    await source.close(); source = await host(join(dir,'source'),url);
    const restarted = message('start','source',agent,2); ui.send(restarted);
    await until(() => seen.some(m => m.body.operation === restarted.id && m.body.result === 'not-authority'));
    assert.equal(journal('source').assignment.assignedHost,'target', 'service restart cannot resurrect consumed source');
  } finally {
    await source.close(); await target.close(); ui.close(); for (const c of server.clients) c.terminate();
    await new Promise<void>(resolve => server.close(() => resolve())); rmSync(dir,{ recursive:true,force:true });
  }
});

test('Stop and Save retract exact source reservation while destination preflight reply is delayed', { timeout: 20000 }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(),'beehive-move-cancel-')));
  const ownerSecret = newKey(), agentSecret = newKey(), agent = publicKey(agentSecret);
  const genesis = createGenesis(publicKey(ownerSecret),agent,'source');
  for (const name of ['source','target']) provision(join(dir,name), { host:name, ownerSecret,agentSecret,runner:process.execPath,args:[resolve('test/runner.ts')],workspace:dir,mode:'fixture' },genesis);
  const server = await relay(0,publicKey(ownerSecret),join(dir,'relay.json')); const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`;
  const source = await host(join(dir,'source'),url), target = await host(join(dir,'target'),url);
  const socket = [...server.clients][0]!, send = socket.send.bind(socket); const held: Parameters<typeof send>[0][] = [];
  socket.send = ((data: Parameters<typeof send>[0], ...args: unknown[]) => {
    if (open(JSON.parse(String(data)),ownerSecret).type === 'prepared') { held.push(data); return; }
    return (send as Function)(data,...args);
  }) as typeof socket.send;
  const seen: Message[] = []; const ui = connect(url,ownerSecret,m => seen.push(m)); await ui.ready;
  const journal = () => JSON.parse(readFileSync(join(dir,'source','journal.json'),'utf8'));
  async function until(predicate: () => boolean) { for(let i=0;i<500;i++) { if(predicate()) return; await delay(10); } throw Error('Missing cancellation evidence'); }
  const selected = { model:'fixture-model',workspace:dir,profile:'default' };
  try {
    ui.send(message('start','source',agent)); await until(() => journal().phase === 'running');
    const actual = journal().actual;
    for (const [revision,action] of [[1,'save'],[2,'stop']] as const) {
      const move = message('move','source',agent,revision,{ target:'target',targetRevision:0,selection:selected }); ui.send(move);
      await until(() => held.length > 0);
      ui.send(move); await until(() => held.length > 1);
      assert.equal(seen.some(m => m.type === 'receipt' && m.body.operation === move.id),false, 'exact pending retry must not publish a terminal interrupted receipt');
      const invalid = message('stop','source',agent,revision+20); ui.send(invalid);
      await until(() => seen.some(m => m.body.operation === invalid.id && m.body.result === 'revision-conflict'));
      assert.equal(journal().move.request.id,move.id);
      for (const invalid of [message('stop','source',agent,revision,{ unexpected:true }), message('save','source',agent,revision,{ ...selected, model:'unsupported' }), message('start','source',agent,revision)]) {
        ui.send(invalid); await until(() => seen.some(m => m.body.operation === invalid.id && m.type === 'receipt'));
        assert.equal(journal().move.request.id,move.id, 'invalid Stop/Save and late Start must not displace reservation');
        assert.deepEqual(journal().actual,actual);
      }
      const cancel = message(action,'source',agent,revision,action === 'save' ? selected : {}); ui.send(cancel);
      await until(() => seen.some(m => m.body.operation === move.id && String(m.body.result).includes('cancelled before grant')));
      for (const data of held.splice(0)) send(data); await delay(100);
      assert.equal(journal().assignment.assignedHost,'source'); assert.equal(journal().assignment.chain,undefined);
      if(action === 'save') assert.deepEqual(journal().actual,actual);
      else await until(() => journal().phase === 'stopped');
    }
    assert.equal(seen.some(m => m.type === 'grant'),false);
  } finally {
    await source.close(); await target.close(); ui.close(); for(const c of server.clients) c.terminate();
    await new Promise<void>(resolve => server.close(() => resolve())); rmSync(dir,{ recursive:true,force:true });
  }
});
