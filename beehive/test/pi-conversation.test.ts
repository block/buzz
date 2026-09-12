import './pi-isolation-loader.mjs';
import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { once } from 'node:events';
import { mkdtempSync, writeFileSync, readFileSync, realpathSync, rmSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
const {ConversationSession}=await import('../src/broker.ts');
const {newKey,publicKey}=await import('../src/protocol.ts');
for(const variant of ['openai','databricks','cancel']) test(`Pi ${variant} conversation broker proves model/effort, native metadata and owned Stop`,async t=>{
 const db=variant!=='openai',cancel=variant==='cancel';
 const root=realpathSync(mkdtempSync(join(tmpdir(),'beehive-pi-conversation-')));t.after(()=>rmSync(root,{recursive:true,force:true}));
 const model=db?'databricks-gpt-5-4':'gpt-5',effort='high',instructions='Pi exact native standing instructions';
 const key={service:'beehive' as const,account:'provider:00000000-0000-0000-0000-000000000000'};
 const observed:any[]=[];let requested!:()=>void;const requestStarted=new Promise<void>(resolve=>{requested=resolve;});
 const server=createServer(async(req,res)=>{let body='';for await(const c of req)body+=c;observed.push({path:req.url,auth:req.headers.authorization,body:JSON.parse(body)});requested();if(!cancel)res.end('{}');});
 server.listen(0,'127.0.0.1');await once(server,'listening');t.after(()=>new Promise<void>(r=>server.close(()=>r())));
 const endpoint=`http://127.0.0.1:${(server.address() as any).port}`,route=db?'/ai-gateway/openai/v1/responses':'/v1/responses';
 writeFileSync(join(root,'pi-fixture.json'),JSON.stringify({model,effort,db,endpoint,route,key,instructions}));
 const executable=join(root,'buzz-pi-acp'),runtime=join(root,'fake-relay-runtime');
 writeFileSync(executable,`#!${process.execPath}\nawait import(${JSON.stringify(new URL('./pi-acp-fixture.ts',import.meta.url).href)});\n`,{mode:0o700});
 writeFileSync(runtime,`#!${process.execPath}
import {spawn} from 'node:child_process';import {createInterface} from 'node:readline';import {writeFileSync} from 'node:fs';
if(process.env.BEEHIVE_PI_KEY || process.env.BEEHIVE_PI_RUNTIME || process.env.BUZZ_ACP_MODEL) throw Error('Relay runtime received provider control');
const child=spawn(process.env.BUZZ_ACP_AGENT_COMMAND,process.env.BUZZ_ACP_AGENT_ARGS.split(','),{detached:true,stdio:['pipe','pipe','ignore'],env:{PATH:'/usr/bin:/bin'}});
writeFileSync('pi-transport-pids',JSON.stringify([process.pid,child.pid]));
const pending=new Map();let seq=0;
createInterface({input:child.stdout}).on('line',line=>{const m=JSON.parse(line);pending.get(m.id)?.(m);});
const request=(method,params)=>new Promise(resolve=>{const id=++seq;pending.set(id,resolve);child.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\\n');});
await request('initialize',{protocolVersion:2,clientCapabilities:{}});
const created=await request('session/new',{cwd:process.cwd(),mcpServers:[]});
await request('session/prompt',{sessionId:created.result.sessionId,prompt:[{type:'text',text:'Synthetic conversation'}]});
`,{mode:0o700});
 const session=new ConversationSession({executable:runtime,relay:'ws://127.0.0.1:1'},{executable,args:[],workspace:root,home:root,configDirectory:root,databricksHost:'',harness:'pi',piCli:realpathSync(process.execPath),model,instructions,resolvedProviderKey:'synthetic-key',buzzProvider:{provider:db?'databricks_v2':'openai-compat',baseUrl:db?'https://pi-workspace.example':'https://api.openai.com/v1',credential:key,models:[model],effort,...(db?{auth:'token' as const}:{wire:'auto' as const})}},newKey(),publicKey(newKey()));
 t.after(()=>session.stop());
 const proof=session.verify();
 if(cancel) {
   const rejected=assert.rejects(proof);await Promise.race([requestStarted,proof]);session.cancel();await rejected;
 } else {
 const evidence=await proof;assert.equal(evidence.model,model);assert.equal(evidence.source,'external-buzz-conversation');
 assert.equal(observed.length,db?2:1);
 for(const [i,row]of observed.entries()){assert.equal(row.path,route);assert.equal(row.body.model,model);assert.equal(row.body.reasoning.effort,effort);assert.equal(row.auth,db?`Bearer synthetic-pi-generation-${i+1}`:'Bearer synthetic-key');}
 }
 await session.stop();
 const owned=JSON.parse(readFileSync(join(root,'pi-owned.json'),'utf8'));
 const transport=JSON.parse(readFileSync(join(root,'pi-transport-pids'),'utf8'));
 for(const pid of [...transport,owned.adapter,owned.descendant])assert.throws(()=>process.kill(pid,0),(e:any)=>e.code==='ESRCH');
 assert.equal(existsSync(owned.directory),false);
 if(db)await assert.rejects(fetch(owned.endpoint+'/responses'));
});
