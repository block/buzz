import test from 'node:test';
import assert from 'node:assert/strict';
import { databricksRuntime } from '../src/databricks-runtime.ts';
import { runtimeEfforts } from '../src/runtime-effort.ts';
import { compatibleCodex, discoverHarnesses, harnessProbe } from '../src/harness-discovery.ts';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync, realpathSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { databricksNative } from '../src/databricks.ts';
import { validateSettings, type Settings } from '../src/settings.ts';
import { runtimeForm } from '../src/runtime-form.ts';
import type { ManagerSnapshot } from '../src/manager-controller.ts';
import { AgentSession } from '../src/acp.ts';
import { prepareConversation } from '../src/conversation.ts';
import { fileURLToPath } from 'node:url';

const key = { service: 'beehive' as const, account: 'provider:00000000-0000-0000-0000-000000000000' };
const input = { host: 'https://first.example', key, model: 'databricks-gpt-5-4' };
const route = '/ai-gateway/openai/v1/responses';
const request = (bridge: { endpoint: string; capability: string }, model = input.model, path = route) => fetch(bridge.endpoint + path, { method: 'POST', headers: { authorization: `Bearer ${bridge.capability}` }, body: JSON.stringify({ model, input: 'synthetic request', reasoning: { effort: 'high' } }) });
const fixture = (t: test.TestContext) => { const p = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-runtime-fixture-'))); t.after(() => rmSync(p, { recursive:true, force:true })); return p; };

test('ongoing Databricks transport refreshes between requests; exact workspace/model survives and revocation fails closed', async t => {
  let expiry = false, revoked = false, reads = 0;
  const observed: string[] = [];
  const bridge = await databricksRuntime(input, async (call, signal) => {
    assert.equal(call.host, input.host); assert.deepEqual(call.key, key); assert.equal(call.action, 'token'); signal.throwIfAborted(); reads++;
    if (revoked) throw Error('synthetic refresh revoked');
    return { ok:true, secret: expiry ? 'synthetic-rotated' : 'synthetic-original' };
  }, async (url, options) => {
    assert.equal(String(url), input.host + route); assert.equal(JSON.parse(String(options?.body)).model, input.model);
    assert.equal(JSON.parse(String(options?.body)).reasoning.effort, 'high'); assert.equal(options?.redirect, 'error');
    observed.push(new Headers(options?.headers).get('authorization')!);
    return new Response('{"output":[]}');
  });
  t.after(() => bridge.stop());
  assert.equal((await request(bridge)).status, 200);
  expiry = true;
  assert.equal((await request(bridge)).status, 200);
  assert.deepEqual(observed, ['Bearer synthetic-original', 'Bearer synthetic-rotated']);
  revoked = true;
  const denied = await request(bridge); assert.equal(denied.status, 503); assert.match(await denied.text(), /sign-in/);
  assert.equal(observed.length, 2); assert.equal(reads, 3);
  assert.equal((await request(bridge, 'other-model')).status, 403);
  assert.equal((await request(bridge, input.model, '/oidc/token')).status, 403);
  assert.equal((await request({ ...bridge, capability: 'foreign-runtime' })).status, 403);
  assert.equal(reads, 3);
});

test('concurrent runtimes isolate capabilities/workspaces; upstream auth rejection latches without fallback', async t => {
  const calls: string[] = [];
  const make = async (host: string) => databricksRuntime({ ...input, host }, async call => ({ ok:true, secret: call.host === input.host ? 'first-token' : 'second-token' }), async (url, options) => {
    calls.push(`${url}:${new Headers(options?.headers).get('authorization')}`);
    return new Response('withheld upstream diagnostic', { status: String(url).startsWith(input.host) ? 401 : 200 });
  });
  const a = await make(input.host), b = await make('https://second.example'); t.after(async () => { await a.stop(); await b.stop(); });
  assert.equal((await request({ endpoint:a.endpoint, capability:b.capability })).status, 403);
  const responses = await Promise.all([request(a), request(b)]);
  assert.deepEqual(responses.map(r => r.status), [401,200]);
  assert.ok(!(await responses[0]!.text()).includes('withheld'));
  assert.equal((await request(a)).status, 401); assert.equal(calls.length, 2);
  assert.ok(calls.includes(input.host + route + ':Bearer first-token'));
  assert.ok(calls.includes('https://second.example' + route + ':Bearer second-token'));
});

test('Stop fences delayed native completion and aborts an ongoing upstream request', async () => {
  let enter!: () => void, finish!: () => void;
  const entered = new Promise<void>(r => enter = r), released = new Promise<void>(r => finish = r);
  let aborted = false, fetched = false;
  const bridge = await databricksRuntime(input, async (_call, signal) => { enter(); signal.addEventListener('abort', () => aborted = true); await released; return { ok:true, secret:'late-token' }; }, async () => { fetched = true; throw Error(); });
  const pending = request(bridge).catch(() => undefined); await entered;
  let stopped = false; const stop = bridge.stop().then(() => stopped = true);
  await Promise.resolve(); assert.equal(aborted,true); assert.equal(stopped,false);
  finish(); await stop; await pending; assert.equal(fetched,false);
  await assert.rejects(request(bridge));
  let fetching!: () => void; const started = new Promise<void>(r => fetching = r);
  const second = await databricksRuntime(input, async () => ({ok:true, secret:'fake'}), async (_url, options) => new Promise((_resolve,reject) => { fetching(); options!.signal!.addEventListener('abort', () => reject(Error('cancelled'))); }));
  const network = request(second).catch(() => undefined); await started; await second.stop(); await network;
});

test('runtime Stop cancels an actual owned native helper, with no OS-store/browser access', async t => {
  const root = fixture(t), helper = join(root,'helper');
  writeFileSync(helper, '#!/bin/sh\nexec /bin/sleep 60\n', { mode:0o700 });
  let enter!: () => void; const entered = new Promise<void>(r => enter = r);
  const bridge = await databricksRuntime(input, (call, signal) => { const result = databricksNative(call, signal, helper); enter(); return result; }, async () => { throw Error('network forbidden'); });
  const pending = request(bridge).catch(() => undefined); await entered; await bridge.stop(); await pending;
});

test('canonical catalogs preserve query paths; no redirects, provider diagnostics, or arbitrary proxy destinations', async t => {
  const paths: string[] = [];
  const bridge = await databricksRuntime(input, async () => ({ok:true, secret:'fake'}), async url => { paths.push(String(url)); return new Response('{"endpoints":[]}'); });
  t.after(() => bridge.stop());
  for (const path of ['/api/ai-gateway/v2/endpoints?page_size=100', '/api/2.1/unity-catalog/model-services?page_size=100&view=FULL&page_token=next']) assert.equal((await fetch(bridge.endpoint+path, {headers:{authorization:`Bearer ${bridge.capability}`}})).status,200);
  assert.deepEqual(paths, [input.host+'/api/ai-gateway/v2/endpoints?page_size=100', input.host+'/api/2.1/unity-catalog/model-services?page_size=100&view=FULL&page_token=next']);
});

test('Desktop contract detects fake adapter/CLI independently, strict Codex version and current Pi command', async t => {
  const root = fixture(t);
  const make = (name: string, output = '') => { const p=join(root,name); writeFileSync(p, `#!/bin/sh\nprintf '%s\\n' '${output}'\n`, {mode:0o700}); return p; };
  make('buzz-agent'); make('buzz-pi-acp'); make('pi'); make('codex-acp','@agentclientprotocol/codex-acp 1.10.0'); make('codex'); make('claude-agent-acp');
  const rows = await discoverHarnesses(new AbortController().signal, {home:root,path:root,bundled:[],common:[],loginShells:[]});
  assert.equal(rows.find(r => r.id === 'pi')?.executable,join(root,'buzz-pi-acp'));
  assert.equal(rows.find(r => r.id === 'pi')?.state,'available');
  assert.deepEqual(rows.find(r => r.id === 'pi')?.providers,['openai','databricks_v2']);
  assert.equal(rows.find(r => r.id === 'codex')?.state,'available');
  assert.equal(rows.find(r => r.id === 'claude')?.state,'cli-missing');
  assert.equal(rows.find(r => r.id === 'goose')?.state,'not-installed');
  assert.deepEqual(rows.filter(r => r.providers.length).map(r => r.id),['codex','buzz-agent','pi']);
  for (const value of ['1.9.99','1.10','1.10.0-rc1','not-version']) assert.equal(compatibleCodex(value),false);
  for (const value of ['1.10.0','pkg 2.0.0']) assert.equal(compatibleCodex(value),true);
  const slow = join(root,'slow'); writeFileSync(slow,'#!/bin/sh\nexec /bin/sleep 60\n',{mode:0o700});
  assert.equal(await harnessProbe(slow,['--version'],root,root,new AbortController().signal,30),undefined);
});

test('effort is manifest-backed, validates persisted settings and reaches actual ACP harness environment', async t => {
  assert.equal(readFileSync(new URL('../src/model-capabilities.json',import.meta.url),'utf8'),readFileSync(new URL('../../scripts/model-capabilities.json',import.meta.url),'utf8'));
  assert.ok(runtimeEfforts('openai','gpt-5').includes('high'));
  assert.deepEqual(runtimeEfforts('openai','unknown-custom'),[]);
  assert.deepEqual(runtimeEfforts('databricks_v2','catalog.schema.gpt-5'),[]);
  assert.ok(!runtimeEfforts('databricks_v2','databricks-claude-sonnet-4-5').includes('none'));
  const root = fixture(t), runner=join(root,'agent');
  writeFileSync(runner, `#!/bin/sh\n[ "$BUZZ_AGENT_THINKING_EFFORT" = high ] || exit 9\nexec '${process.execPath}' '${fileURLToPath(new URL('./acp-fixture.ts',import.meta.url))}' openai\n`, {mode:0o700});
  const settings: Settings = {version:1,revision:1,agents:[],providers:[{id:'p',name:'OpenAI',type:'openai',endpoint:'https://api.openai.com/v1',key}],runtimes:[{id:'r',name:'Reasoning',harness:'buzz-agent',executable:runner,providerId:'p',model:'gpt-5',effort:'high'}]};
  validateSettings(settings);
  assert.throws(() => validateSettings({...settings,runtimes:[{...settings.runtimes[0]!,effort:'banana'}]}),/Effort/);
  // Actual runtime projection is also covered by the host Save/Start fixture.
  const session = new AgentSession({ executable:runner,args:[],workspace:root,home:root,configDirectory:root,databricksHost:'',model:'gpt-5',resolvedProviderKey:'synthetic-provider-key',buzzProvider:{provider:'openai-compat',baseUrl:settings.providers[0]!.endpoint,wire:'auto',credential:key,models:['gpt-5'],effort:'high'} });
  t.after(() => session.owned.stop());
  await session.catalog(); await session.verify();
});

test('short runtime form selects supported model-specific effort; unknown custom omits it and unsupported harness cannot save', async () => {
  const snapshot: ManagerSnapshot = { local:[], agents:[], status:'synthetic', models:['gpt-5'], harnesses:[{id:'buzz-agent',label:'Buzz Agent',executable:'/fixture',state:'available',providers:['openai'],reason:'fixture'},{id:'pi',label:'Pi',state:'available',providers:[],reason:'Provider integration unavailable'}], settings:{version:1,revision:1,agents:[],runtimes:[],providers:[{id:'p',name:'OpenAI',type:'openai',endpoint:'https://api.openai.com/v1',key}]} };
  const submitted: {action:string;values?:Record<string,string>}[] = [];
  const run = async (choices: string[], inputs: string[]) => runtimeForm({choose:async (_label,options) => {const value=choices.shift(); assert.ok(value === undefined || options.includes(value)); return value;},input:async () => inputs.shift(),confirm:async label => {assert.match(label,/Running agents do not change/);return true;},notice:() => {}}, () => snapshot, async (action,values) => {submitted.push({action,values});});
  await run(['Buzz Agent · Available','OpenAI · p','gpt-5','high'],['Reasoning']);
  assert.equal(submitted.at(-1)?.values?.effort,'high');
  assert.equal(submitted.at(-1)?.values?.model,'gpt-5');
  submitted.length=0;
  await run(['Buzz Agent · Available','OpenAI · p','Custom model'],['unrecognized-model','Custom']);
  assert.equal(submitted.at(-1)?.action,'add-runtime'); assert.equal(submitted.at(-1)?.values?.effort,undefined);
  submitted.length=0;
  await run(['Pi · Provider integration unavailable'],[]);
  assert.deepEqual(submitted.map(s => s.action),['runtime-form']);
});

test('one actual wrapper/harness run makes requests before expiry, after refresh, and after revocation without restart', async t => {
  const root = fixture(t), loader = join(root,'isolation.mjs'), harness=join(root,'harness.mjs'), executable=join(root,'entry');
  // This explicit loader is passed only to the fixture wrapper process. It blocks
  // native custody completely and rejects every nonfixture upstream destination.
  writeFileSync(loader, `import { registerHooks } from 'node:module';
let tokens=0;
registerHooks({load(url,context,next) {const result=next(url,context); if(url.endsWith('/src/databricks.ts')) return {...result,source: "export function databricksHost(s) { if(s !== 'https://first.example') throw Error(); return s; } export async function databricksNative(input,signal) { signal.throwIfAborted(); if(input.action !== 'token') throw Error(); return globalThis.fixtureToken(); }"}; return result;}});
globalThis.fixtureToken=() => { if(++tokens===3) throw Error('synthetic revoked'); return {ok:true,secret:tokens===1?'fixture-before':'fixture-after'}; };
globalThis.fetch=async (url,options) => { if(url !== 'https://first.example/ai-gateway/openai/v1/responses') throw Error('network forbidden'); const bearer=new Headers(options.headers).get('authorization'); if(bearer !== (tokens===1?'Bearer fixture-before':'Bearer fixture-after')) throw Error('wrong refreshed token'); return new Response(JSON.stringify({output:[],token:tokens})); };
`);
  writeFileSync(harness, `import {createInterface} from 'node:readline';
const endpoint=process.env.DATABRICKS_HOST, capability=process.env.DATABRICKS_TOKEN;
if(!/^http:\\/\\/127\\.0\\.0\\.1:\\d+$/.test(endpoint) || !/^[0-9a-f]{64}$/.test(capability) || process.env.BEEHIVE_DATABRICKS_RUNTIME) process.exit(8);
createInterface({input:process.stdin}).on('line',async line=>{const m=JSON.parse(line);let result;
if(m.method==='initialize') result={protocolVersion:2,agentInfo:{name:'buzz-agent'}};
else if(m.method==='session/new') result={sessionId:'same-run',models:{availableModels:[{modelId:'databricks-gpt-5-4'}]}};
else if(m.method==='session/set_model') result={sessionId:'same-run',modelId:'databricks-gpt-5-4'};
else if(m.method==='session/prompt') {for(let i=1;i<=3;i++){const r=await fetch(endpoint+'/ai-gateway/openai/v1/responses',{method:'POST',headers:{authorization:'Bearer '+capability},body:JSON.stringify({model:'databricks-gpt-5-4',input:'fake'})}); if(r.status !== (i===3?503:200)) process.exit(9);if(i<3 && (await r.json()).token !== i) process.exit(10);} console.log(JSON.stringify({jsonrpc:'2.0',method:'session/update',params:{sessionId:'same-run',update:{sessionUpdate:'agent_message_chunk',content:{type:'text',text:'one run refreshed and refused revoked access'}}}}));result={stopReason:'end_turn'};}
console.log(JSON.stringify({jsonrpc:'2.0',id:m.id,result}));});
`);
  const wrapper=fileURLToPath(new URL('../src/databricks-runtime-child.ts',import.meta.url));
  // Public reference only in shell/argv; fake bearers are created exclusively in
  // the isolated loader. No retained config, OS credentials or real harness.
  writeFileSync(executable, `#!/bin/sh\nexport BEEHIVE_DATABRICKS_RUNTIME='${JSON.stringify(input)}'\nexec '${process.execPath}' --import '${loader}' '${wrapper}' '${process.execPath}' '${harness}'\n`, {mode:0o700});
  const session=new AgentSession({executable,args:[],workspace:root,home:root,configDirectory:root,databricksHost:input.host,model:input.model});
  t.after(() => session.owned.stop());
  await session.catalog(); const evidence=await session.verify(); assert.equal(evidence.session,'same-run'); assert.equal(evidence.model,input.model);
});

test('relay runtime never receives the OS-backed Databricks bearer', t => {
  const root=fixture(t);
  const plan=prepareConversation({executable:realpathSync(process.execPath),relay:'ws://127.0.0.1:1'}, {executable:realpathSync(process.execPath),args:[],workspace:root,home:root,configDirectory:root,databricksHost:'',model:input.model,resolvedProviderKey:'synthetic-provider-token',buzzProvider:{provider:'databricks_v2',auth:'token',baseUrl:input.host,credential:key,models:[input.model]}}, '1'.repeat(64), '2'.repeat(64));
  assert.equal(plan.env.DATABRICKS_TOKEN,undefined);
  assert.ok(!JSON.stringify(plan.env).includes('synthetic-provider-token'));
});


test('Codex saved-provider matrix rejects Databricks, missing CLI and invented generic effort', () => {
  const settings: Settings = {version:1,revision:0,agents:[],providers:[{id:'p',name:'OpenAI',type:'openai',endpoint:'https://api.openai.com/v1',key}],runtimes:[{id:'r',name:'Custom Codex',harness:'codex',cli:process.execPath,executable:process.execPath,providerId:'p',model:'custom-codex-model'}]};
  assert.equal(validateSettings(settings).runtimes[0]!.model,'custom-codex-model');
  assert.throws(() => validateSettings({...settings,runtimes:[{...settings.runtimes[0]!,cli:undefined}]}),/combination/);
  assert.throws(() => validateSettings({...settings,runtimes:[{...settings.runtimes[0]!,effort:'high'}]}),/combination/);
  assert.throws(() => validateSettings({...settings,providers:[{...settings.providers[0]!,type:'databricks_v2',endpoint:'https://fixture.example'}]}),/combination/);
});
