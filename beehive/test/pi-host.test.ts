import './pi-isolation-loader.mjs';
import type { Setup } from '../src/host.ts';
import type { Message } from '../src/protocol.ts';
import { createServer } from 'node:http';
import { once } from 'node:events';
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, existsSync, readFileSync, realpathSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
// Load product modules only AFTER the explicit subprocess isolation hooks.
const { addDatabricks } = await import('../src/databricks.ts');
const { addOpenAI } = await import('../src/settings-credentials.ts');
const { readSettings, saveSettings, settingsId } = await import('../src/settings.ts');
const { newKey, publicKey, message } = await import('../src/protocol.ts');
const { host, provision } = await import('../src/host.ts');
const { createGenesis } = await import('../src/assignment.ts');
const { writePrivate } = await import('../src/storage.ts');

function fixture(t: test.TestContext) { const root = realpathSync(mkdtempSync(join(tmpdir(),'beehive-pairing-cli-settings-'))); t.after(() => rmSync(root,{ recursive: true, force: true })); return root; }
const sleep = (ms: number) => new Promise(r => setTimeout(r,ms));
async function wait(fn: () => boolean) { for(let n=0;n<150;n++) { if(fn()) return; await sleep(20); } throw Error('Fixture wait expired'); }

for (const providerType of ['pi-openai','pi-databricks','pi-custom','pi-databricks-custom'] as const) test(`${providerType} saved runtime reaches host Save/new Start while the prior active run remains immutable`, async t => {
  const root = fixture(t), owner = newKey(), agent = newKey(), hostKey = publicKey(newKey());
  const db = providerType.startsWith('pi-databricks');
  const home = root, config = root;
  const setup: Setup = { host: hostKey, ownerSecret: owner, agentSecret: agent, runner: realpathSync(process.execPath), args: ['-e','setInterval(()=>{},1000)'], workspace: root, serviceHome: home, configDirectory: config, mode: 'fixture' };
  provision(root,setup,createGenesis(publicKey(owner),publicKey(agent),hostKey));
  let receive: (m: Message) => void = () => {}; const reports: Message[] = [];
  const transport = { binding: { host: hostKey, owner: publicKey(owner) }, validate() {}, connect(_url: string,_secret: string,handle: (m: Message) => void) { receive = handle; return { ready: Promise.resolve(), send(m: Message) { reports.push(m); }, close() {} }; } };
  let providerReads = 0;
  const running = await host(root,'ws://127.0.0.1',undefined,transport,undefined,async (input,signal) => { signal.throwIfAborted(); assert.equal((input as any).provider,!db ? 'openai-compat' : 'databricks_v2'); if (db) assert.equal((input as any).host,'https://pi-workspace.example'); providerReads++; return {ok:true,secret:'synthetic-provider-key'}; });
  t.after(() => running.close());
  const start = message('start',hostKey,publicKey(agent),0); receive(start);
  await wait(() => reports.some(m => m.type === 'inventory' && m.body.phase === 'running'));
  const active = reports.filter(m => m.type === 'inventory').at(-1)!.body.actualRun;
  let stored: string | null = null;
  if (!db) addOpenAI(root,'Fixture','synthetic',{ read: () => stored, create(_r,v) { stored = v; } });
  else await addDatabricks(root,'Workspace','https://pi-workspace.example',new AbortController().signal,async input => { assert.equal(input.action,'login'); return {ok:true}; });
  const prior = readSettings(root), id = settingsId();
  const model = providerType === 'pi-databricks-custom' ? 'tenant.schema.gpt-6-service' : providerType === 'pi-custom' ? 'retained-custom-model' : db ? 'databricks-gpt-5-4' : 'gpt-5';
  const effort = providerType.endsWith('custom') ? undefined : 'high';
  const executable = join(root,'buzz-pi-acp');
  const observations: any[] = [];
  const server = createServer(async (req,res) => {
    let body=''; for await(const chunk of req) body+=chunk;
    observations.push({path:req.url,authorization:req.headers.authorization,body:JSON.parse(body)});
    res.setHeader('content-type','text/event-stream');res.end('data: {"type":"response.completed"}\n\n');
  });
  server.listen(0,'127.0.0.1');await once(server,'listening');
  t.after(() => new Promise<void>(resolve=>server.close(()=>resolve())));
  const endpoint=`http://127.0.0.1:${(server.address() as any).port}`;
  const route=db?'/ai-gateway/openai/v1/responses':'/v1/responses';
  writeFileSync(join(root,'pi-fixture.json'),JSON.stringify({model,effort,db,endpoint,route,key:prior.providers[0]!.key}));
  writeFileSync(executable,`#!${process.execPath}\nawait import(${JSON.stringify(new URL('./pi-acp-fixture.ts',import.meta.url).href)});\n`,{mode:0o700});
  saveSettings(root,{ ...prior, runtimes: [{ id, name: 'Pi custom name', harness: 'pi', executable, providerId: prior.providers[0]!.id, model, cli:realpathSync(process.execPath), ...(effort?{effort}:{}) }] },prior.revision);
  assert.equal(readSettings(root).runtimes[0]!.name,'Pi custom name');
  assert.equal(providerReads,0,'Save does not accept secrets or start Pi');
  await wait(() => running.settingsRevision === 2);
  await wait(() => reports.some(m => m.type === 'inventory' && (m.body.harnessSetups as any[]).some(r => r.id === `runtime:${id}`)));
  const inventory = reports.filter(m => m.type === 'inventory').at(-1)!;
  assert.deepEqual(inventory.body.actualRun,active);
  const binding = (inventory.body.harnessSetups as any[]).find(r => r.id === `runtime:${id}`);
  const save = message('save',hostKey,publicKey(agent),inventory.revision,{ model, workspace: root, profile: 'default', harnessSetup: { id: binding.id, fingerprint: binding.fingerprint } });
  receive(save); await wait(() => reports.some(m => m.body.operation === save.id));
  assert.equal(reports.find(m => m.body.operation === save.id)?.body.result,'saved; running configuration unchanged');
  assert.deepEqual(reports.filter(m => m.type === 'inventory').at(-1)!.body.actualRun,active);
  const current = reports.filter(m => m.type === 'inventory').at(-1)!;
  const stop = message('stop',hostKey,publicKey(agent),current.revision); receive(stop); await wait(() => reports.some(m => m.body.operation === stop.id));
  const stopped = reports.filter(m => m.type === 'inventory').at(-1)!;
  assert.equal(stopped.body.phase,'stopped');
  const nextStart = message('start',hostKey,publicKey(agent),stopped.revision); receive(nextStart); await wait(() => reports.some(m => m.body.operation === nextStart.id));
  assert.equal(reports.find(m => m.body.operation === nextStart.id)?.body.result,'accepted');
  assert.equal((reports.filter(m => m.type === 'inventory').at(-1)!.body.actualRun as any).selection.model,model);
  assert.equal(providerReads,1);
  assert.equal(observations.length,db?2:1);
  for(const [i,observed] of observations.entries()) {
    assert.equal(observed.path,route);
    assert.equal(observed.body.model,model);
    assert.equal(observed.body.reasoning?.effort,effort);
    assert.equal(observed.authorization,db?`Bearer synthetic-pi-generation-${i+1}`:'Bearer synthetic-provider-key');
  }
  const owned=JSON.parse(readFileSync(join(root,'pi-owned.json'),'utf8'));
  const beforeStop=reports.filter(m=>m.type==='inventory').at(-1)!;
  const piStop=message('stop',hostKey,publicKey(agent),beforeStop.revision);receive(piStop);
  await wait(()=>reports.some(m=>m.body.operation===piStop.id));
  assert.equal(reports.filter(m=>m.type==='inventory').at(-1)!.body.phase,'stopped');
  for(const pid of [owned.adapter,owned.descendant]) assert.throws(()=>process.kill(pid,0),(e:any)=>e.code==='ESRCH');
  assert.equal(existsSync(owned.directory),false);
  if(db) { await assert.rejects(fetch(owned.endpoint+'/responses')); const native=readFileSync(join(root,'native-calls'),'utf8').trim().split('\n').map(v=>JSON.parse(v)); assert.equal(native.length,2); assert.deepEqual(native.map(v=>v.input.key),[prior.providers[0]!.key,prior.providers[0]!.key]); }
  assert.ok(!JSON.stringify(reports).includes('synthetic-provider-key'));
  assert.ok(!readFileSync(join(root,'journal.json'),'utf8').includes('synthetic-provider-key'));
  const invalid = { ...readSettings(root), revision: 3, runtimes: [] }; writePrivate(join(root,'settings.json'),invalid);
  await sleep(2100); assert.equal(running.settingsRevision,2,'invalid replacement retains effective catalog');
  await running.close();
});

