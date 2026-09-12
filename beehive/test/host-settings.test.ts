import { addDatabricks } from '../src/databricks.ts';
import { wrapManagement, unwrapManagement } from '../src/nostr-codec.ts';
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, existsSync, readFileSync, realpathSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { tmpdir } from 'node:os';
import { finalizeEvent } from 'nostr-tools/pure';
import { nsecEncode } from 'nostr-tools/nip19';
import { registerAgent, agentNsec, addOpenAI } from '../src/settings-credentials.ts';
import { readSettings, saveSettings, settingsId } from '../src/settings.ts';
import { profileFromEvents } from '../src/agent-profile.ts';
import { isolatedFileCredentials } from './isolated-file-credentials.ts';
import { newKey, publicKey, message, serializeManagement, seal, open, type Message } from '../src/protocol.ts';
import { openAIModels } from '../src/settings-models.ts';
import { host, provision, type Setup, bindingFingerprint } from '../src/host.ts';
import { createGenesis } from '../src/assignment.ts';
import { writePrivate } from '../src/storage.ts';

function fixture(t: test.TestContext) { const root = realpathSync(mkdtempSync(join(tmpdir(),'beehive-pairing-cli-settings-'))); t.after(() => rmSync(root,{ recursive: true, force: true })); return root; }
const sleep = (ms: number) => new Promise(r => setTimeout(r,ms));
async function wait(fn: () => boolean) { for(let n=0;n<150;n++) { if(fn()) return; await sleep(20); } throw Error('Fixture wait expired'); }

test('identity-only nsec registration verifies retained keys and preserves public-only state', t => {
  const root = fixture(t), credentials = isolatedFileCredentials(join(root,'keys.json')), secret = newKey();
  assert.equal(agentNsec(nsecEncode(Buffer.from(secret,'hex'))),secret);
  assert.throws(() => agentNsec(secret),/nsec/);
  const row = registerAgent(root,secret,credentials,{ profileState: 'none' });
  assert.equal(row.publicKey,publicKey(secret));
  assert.equal(existsSync(join(root,'setup.json')),false);
  assert.equal(existsSync(join(root,'journal.json')),false);
  assert.equal(readSettings(root).providers.length,0);
  assert.equal(readSettings(root).runtimes.length,0);
  assert.ok(!readFileSync(join(root,'settings.json'),'utf8').includes(secret));
  registerAgent(root,secret,credentials,{ profileState: 'unavailable' });
  assert.equal(readSettings(root).revision,1);
  credentials.remove(row.key);
  assert.throws(() => registerAgent(root,secret,credentials,{ profileState: 'none' }),/missing/);
  assert.equal(readSettings(root).revision,1);
  const other = newKey();
  assert.throws(() => registerAgent(root,other,{ read: () => null, create() {}, remove() {} },{ profileState: 'none' }),/missing/);
  assert.equal(readSettings(root).agents.length,1);
  assert.ok(readFileSync(join(root,'credential-attempts.json'),'utf8').includes(publicKey(other)));
});

test('signed profile selects valid latest metadata, strips controls and does not invent names', () => {
  const secret = newKey(), author = publicKey(secret), now = Math.floor(Date.now()/1000);
  const event = finalizeEvent({ kind: 0, created_at: now, tags: [], content: JSON.stringify({ name: 'Fixture\u001b[31m', about: 'hello' }) },Buffer.from(secret,'hex'));
  assert.equal(profileFromEvents([event],author,'wss://fixture.invalid').profileState,'found');
  assert.equal(profileFromEvents([event],publicKey(newKey()),'wss://fixture.invalid').profileState,'none');
  assert.equal(profileFromEvents([{ ...event, content: '{}' }],author,'wss://fixture.invalid').profileState,'none');
  const empty = finalizeEvent({ kind: 0, created_at: now, tags: [], content: '{}' },Buffer.from(secret,'hex'));
  assert.equal(profileFromEvents([empty],author,'wss://fixture.invalid').profile?.name,undefined);
});

test('OpenAI verifies store before public commit; listing is bounded and custom is independent', async t => {
  const root = fixture(t); let stored: string | null = null;
  assert.throws(() => addOpenAI(root,'Fixture','synthetic',{ read: () => null, create() {} }),/verification/);
  assert.equal(readSettings(root).providers.length,0);
  addOpenAI(root,'Fixture','synthetic',{ read: () => stored, create(_ref,value) { stored = value; } });
  const catalog = readSettings(root), provider = catalog.providers[0]!;
  assert.ok(!readFileSync(join(root,'settings.json'),'utf8').includes('synthetic'));
  const models = await openAIModels('synthetic',new AbortController().signal,async (_url,options) => {
    assert.equal((options?.headers as Record<string,string>).Authorization,'Bearer synthetic');
    return new Response(JSON.stringify({ data: [{ id: 'gpt-5' },{ id: 'gpt-5-2026-01-01' },{ id: 'text-embedding-3-small' }] }));
  });
  assert.deepEqual(models,['gpt-5']);
  await assert.rejects(openAIModels('synthetic',new AbortController().signal,async () => new Response('',{ status: 403 })),/unavailable/);
  saveSettings(root,{ ...catalog,runtimes: [{ id: settingsId(), name: 'Custom', harness: 'buzz-agent', executable: process.execPath, providerId: provider.id, model: 'unknown-custom-model' }] },catalog.revision);
  assert.equal(readSettings(root).runtimes[0]?.model,'unknown-custom-model');
  assert.throws(() => saveSettings(root,catalog,catalog.revision),/changed/);
});

for (const providerType of ['openai','databricks_v2'] as const) test(`${providerType} saved runtime reaches host Save/new Start while the prior active run remains immutable`, async t => {
  const root = fixture(t), owner = newKey(), agent = newKey(), hostKey = publicKey(newKey());
  const setup: Setup = { host: hostKey, ownerSecret: owner, agentSecret: agent, runner: realpathSync(process.execPath), args: ['-e','setInterval(()=>{},1000)'], workspace: root, serviceHome: root, configDirectory: root, mode: 'fixture' };
  provision(root,setup,createGenesis(publicKey(owner),publicKey(agent),hostKey));
  let receive: (m: Message) => void = () => {}; const reports: Message[] = [];
  const transport = { binding: { host: hostKey, owner: publicKey(owner) }, validate() {}, connect(_url: string,_secret: string,handle: (m: Message) => void) { receive = handle; return { ready: Promise.resolve(), send(m: Message) { reports.push(m); }, close() {} }; } };
  let providerReads = 0;
  const running = await host(root,'ws://127.0.0.1',undefined,transport,undefined,async (input,signal) => { signal.throwIfAborted(); assert.equal((input as any).provider,providerType === 'openai' ? 'openai-compat' : 'databricks_v2'); if (providerType === 'databricks_v2') assert.equal((input as any).host,'https://fixture.example'); providerReads++; return {ok:true,secret:'synthetic-provider-key'}; });
  t.after(() => running.close());
  const start = message('start',hostKey,publicKey(agent),0); receive(start);
  await wait(() => reports.some(m => m.type === 'inventory' && m.body.phase === 'running'));
  const active = reports.filter(m => m.type === 'inventory').at(-1)!.body.actualRun;
  let stored: string | null = null;
  if (providerType === 'openai') addOpenAI(root,'Fixture','synthetic',{ read: () => stored, create(_r,v) { stored = v; } });
  else await addDatabricks(root,'Fixture','https://fixture.example',new AbortController().signal,async () => ({ok:true}));
  const prior = readSettings(root), id = settingsId();
  const model = providerType === 'openai' ? 'gpt-5' : 'databricks-gpt-5-4';
  const executable = join(root,'fixture-buzz-agent');
  writeFileSync(executable,`#!/bin/sh\n[ \"$BUZZ_AGENT_THINKING_EFFORT\" = high ] || exit 9\nexec '${process.execPath}' '${fileURLToPath(new URL('./acp-fixture.ts',import.meta.url))}' ${providerType === 'openai' ? 'openai' : 'databricks-os'}\n`,{mode:0o700});
  saveSettings(root,{ ...prior, runtimes: [{ id, name: 'Future', harness: 'buzz-agent', executable, providerId: prior.providers[0]!.id, model, effort: 'high' }] },prior.revision);
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
  assert.ok(!JSON.stringify(reports).includes('synthetic-provider-key'));
  assert.ok(!readFileSync(join(root,'journal.json'),'utf8').includes('synthetic-provider-key'));
  const invalid = { ...readSettings(root), revision: 3, runtimes: [] }; writePrivate(join(root,'settings.json'),invalid);
  await sleep(2100); assert.equal(running.settingsRevision,2,'invalid replacement retains effective catalog');
  await running.close();
});

test('oversized projected inventory retains loaded catalog and serializable reports, including on reopen', async t => {
  const root = fixture(t), owner = newKey(), agent = newKey(), hostKey = publicKey(newKey());
  // Long real paths repeat in each wire row; 100 short public records are not a wire budget.
  const setup: Setup = { host: hostKey, ownerSecret: owner, agentSecret: agent, runner: realpathSync(process.execPath), args: [], workspace: root, allowedWorkspaces: [root, ...Array.from({length: 12}, (_, i) => join(root, `workspace-${i}-` + 'x'.repeat(120)))], serviceHome: root, configDirectory: root, mode: 'fixture' };
  provision(root,setup,createGenesis(publicKey(owner),publicKey(agent),hostKey));
  const reports: Message[] = [];
  const transport = { binding: {host: hostKey, owner: publicKey(owner)}, validate() {}, connect() { return { ready: Promise.resolve(), send(m: Message) { serializeManagement(m); reports.push(m); }, close() {} }; } };
  const running = await host(root,'ws://127.0.0.1',undefined,transport);
  t.after(() => running.close());
  let stored: string | null = null;
  addOpenAI(root,'Fixture','synthetic',{read: () => stored, create(_r,v) { stored = v; }});
  const prior = readSettings(root);
  const row = {id: settingsId(), name: 'Fitting', harness: 'buzz-agent' as const, executable: realpathSync(process.execPath), providerId: prior.providers[0]!.id, model: 'm'.repeat(200)};
  saveSettings(root,{...prior, runtimes: [row]},prior.revision);
  await wait(() => running.settingsRevision === 2);
  const fitting = reports.filter(m => m.type === 'inventory').at(-1)!;
  assert.equal((fitting.body.harnessSetups as unknown[]).length,2);
  // Both production codecs accept the actual fitting inventory.
  assert.deepEqual(open(seal(fitting,owner),owner),JSON.parse(serializeManagement(fitting)));
  assert.deepEqual(unwrapManagement(wrapManagement(fitting,Buffer.from(owner,'hex'),publicKey(agent)),Buffer.from(agent,'hex')).message,JSON.parse(serializeManagement(fitting)));
  const saved = readSettings(root);
  saveSettings(root,{...saved,runtimes: [...saved.runtimes,...Array.from({length: 18}, () => ({...row,id:settingsId()}))]},saved.revision);
  assert.equal(readSettings(root).revision,3,'public save is retained, not falsely reported loaded');
  const before = reports.length;
  await sleep(2100);
  assert.equal(running.settingsRevision,2);
  assert.ok(reports.length > before,'prior inventory continues to publish');
  assert.deepEqual(reports.filter(m => m.type === 'inventory').at(-1)!.body.harnessSetups,fitting.body.harnessSetups);
  await running.close();
  await assert.rejects(host(root,'ws://127.0.0.1',undefined,transport),/bounded wire size/);
  assert.equal(existsSync(join(root,'host.lock')),false,'failed startup releases only its own lock');
});

test('shared management serializer preserves the exact UTF-8 wire limit for both producers', () => {
  const secret = newKey(), recipient = newKey();
  const m = message('inventory',publicKey(secret),publicKey(recipient),0,{ padding: '' });
  const overhead = Buffer.byteLength(JSON.stringify(m));
  m.body.padding = 'a'.repeat(32768-overhead);
  assert.equal(Buffer.byteLength(serializeManagement(m)),32768);
  assert.deepEqual(open(seal(m,secret),secret),m);
  assert.deepEqual(unwrapManagement(wrapManagement(m,Buffer.from(secret,'hex'),publicKey(recipient)),Buffer.from(recipient,'hex')).message,m);
  m.body.padding += 'é';
  assert.throws(() => serializeManagement(m),/bounded wire size/);
  assert.throws(() => seal(m,secret),/bounded wire size/);
  assert.throws(() => wrapManagement(m,Buffer.from(secret,'hex'),publicKey(recipient)),/bounded wire size/);
});
