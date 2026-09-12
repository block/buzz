import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, realpathSync, rmSync, readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { AgentSession, prepareAgent, type AgentLaunch } from '../src/acp.ts';
import { piEfforts, piModelConfig } from '../src/pi.ts';
import { runtimeForm } from '../src/runtime-form.ts';
import type { ManagerSnapshot } from '../src/manager-controller.ts';
import { runtimeCapabilities } from '../src/runtime-effort.ts';
const key={service:'beehive' as const,account:'provider:00000000-0000-0000-0000-000000000000'};
const provider={provider:'openai-compat' as const,baseUrl:'https://api.openai.com/v1',wire:'auto' as const,credential:key,models:['gpt-5'],effort:'high'};
for(const mode of ['wrong-model','wrong-effort','wrong-fork','wrong-protocol','drift-effort','drift-model']) test(`Pi rejects ${mode} before any provider request, Stop owns children`,async t=>{
 const root=realpathSync(mkdtempSync(join(tmpdir(),'beehive-pi-negative-')));t.after(()=>rmSync(root,{recursive:true,force:true}));
 const executable=join(root,'buzz-pi-acp');
 writeFileSync(executable,`#!${process.execPath}\nawait import(${JSON.stringify(new URL('./pi-acp-fixture.ts',import.meta.url).href)});\n`,{mode:0o700});
 writeFileSync(join(root,'pi-fixture.json'),JSON.stringify({mode,model:'gpt-5',effort:'high',db:false,endpoint:'http://127.0.0.1:1'}));
 const launch:AgentLaunch={executable,args:[],workspace:root,home:root,configDirectory:root,databricksHost:'',harness:'pi',piCli:realpathSync(process.execPath),buzzProvider:provider,model:'gpt-5',resolvedProviderKey:'synthetic-secret'};
 assert.throws(()=>prepareAgent({...launch,resolvedProviderKey:undefined}),/admission/);
 const session=new AgentSession(launch);let stopped=false;t.after(()=>stopped?undefined:session.owned.stop());
 if(mode.startsWith('drift-')) await session.catalog(); else await assert.rejects(session.catalog());
 await assert.rejects(session.verify());
 await session.owned.stop();stopped=true;
 const owned=JSON.parse(readFileSync(join(root,'pi-owned.json'),'utf8'));
 for(const pid of [owned.adapter,owned.descendant]) assert.throws(()=>process.kill(pid,0),(e:any)=>e.code==='ESRCH');
 assert.equal(existsSync(owned.directory),false);
});

test('Pi model routes use the shared capability owner; UC tenant components never choose protocol or effort',()=>{
 const db={...provider,provider:'databricks_v2' as const,wire:undefined,auth:'token' as const,baseUrl:'https://pi-workspace.example'};
 for(const [model,api,suffix] of [['databricks-gpt-5-4','openai-responses','/ai-gateway/openai/v1'],['databricks-claude-sonnet-4-5','anthropic-messages','/ai-gateway/anthropic'],['gpt-5.schema.unknown','openai-completions','/ai-gateway/mlflow/v1'],['catalog.schema.gpt-6-service','openai-responses','/ai-gateway/openai/v1'],['catalog.schema.agpt-5-gpt-6','openai-responses','/ai-gateway/openai/v1'],['catalog.schema.claude-gpt-5','openai-completions','/ai-gateway/mlflow/v1']]) {
  const p=piModelConfig(db,model).providers.beehive;
  assert.equal(p.api,api);assert.equal(p.baseUrl,db.baseUrl+suffix);assert.equal(p.models[0].id,model);
 }
 assert.deepEqual(piEfforts('databricks_v2','catalog.schema.gpt-6-service'),[]);
 assert.deepEqual(piEfforts('openai','unknown-custom'),[]);
 assert.equal(runtimeCapabilities('databricks_v2','gpt-5.schema.unknown')!.supported_efforts.length,0);
});

test('actual runtime form offers Pi both providers, retains custom names, submits only supported model effort',async()=>{
 for(const type of ['openai','databricks_v2'] as const) {
  const model=type==='openai'?'gpt-5':'databricks-gpt-5-4';
  const snapshot:ManagerSnapshot={local:[],agents:[],status:'fixture',harnesses:[{id:'pi',label:'Pi',executable:'/fake',cli:'/fake-pi',state:'available',providers:['openai','databricks_v2'],reason:'synthetic'}],models:[model],settings:{version:1,revision:1,agents:[],runtimes:[],providers:[{id:'p',name:'Saved',type,endpoint:type==='openai'?'https://api.openai.com/v1':'https://pi-workspace.example',key}]}};
  const choices=['Pi · Available','Saved · p',model,'high'];const calls:any[]=[];
  await runtimeForm({async choose(_label,options){const value=choices.shift();assert.ok(options.includes(value!));return value;},async input(){return 'My Pi';},async confirm(){return true;},notice(){throw Error('unexpected notice');}},()=>snapshot,async(action,values)=>{calls.push({action,values});});
  assert.deepEqual(calls.at(-1),{action:'add-runtime',values:{name:'My Pi',model,provider:'p',harness:'pi',effort:'high'}});
 }
});
