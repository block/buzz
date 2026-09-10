import { test } from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync,realpathSync,readFileSync,rmSync} from 'node:fs';
import {spawn} from 'node:child_process';
import {writePrivate} from '../src/storage.ts';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {setTimeout as delay} from 'node:timers/promises';
import {host,provision} from '../src/host.ts';
import {relay} from '../src/relay.ts';
import {connect} from '../src/client.ts';
import {newKey,publicKey,message,type Message} from '../src/protocol.ts';
import {createGenesis} from '../src/assignment.ts';
import {profileRevision} from '../src/profiles.ts';
test('original reviewer actual TUI profiled Move materializes one truthful named revision across reselect, Restart and reopen', { timeout: 30000 }, async () => {
const dir=realpathSync(mkdtempSync(join(tmpdir(),'bh-review-named-'))), owner=newKey(), key=newKey(), agent=publicKey(key);
const root=createGenesis(publicKey(owner),agent,'s');
for(const name of ['s','t']) provision(join(dir,name),{host:name,ownerSecret:owner,agentSecret:key,runner:realpathSync(process.execPath),args:[resolve('test/runner.ts')],workspace:dir,mode:'fixture'},root);
const server=await relay(0,publicKey(owner),join(dir,'relay.json')); const address=server.address(); assert.ok(address&&typeof address!=='string');
const url=`ws://127.0.0.1:${address.port}`, seen:Message[]=[];
let s=await host(join(dir,'s'),url), t=await host(join(dir,'t'),url); const c=connect(url,owner,m=>seen.push(m)); await c.ready;
const state=(h:string)=>JSON.parse(readFileSync(join(dir,h,'journal.json'),'utf8'));
async function until(f:()=>boolean){for(let i=0;i<500;i++){if(f())return;await delay(20);}throw Error('probe deadline');}
async function send(type:Message['type'],h:string,body:any={},revision=state(h).revision){const m=message(type,h,agent,revision,body);c.send(m);await until(()=>seen.some(r=>r.type==='receipt'&&r.body.operation===m.id));return {m,r:seen.find(r=>r.type==='receipt'&&r.body.operation===m.id)!};}
try{
 const p={name:'Source',parent:null,instructions:'Reviewer source behavior',revision:profileRevision('Source',null,'Reviewer source behavior')}; c.send(message('profile','profiles','profiles',0,p)); await delay(100);
 assert.match(String((await send('save','s',{...state('s').selected,profile:p.revision,behavior:p})).r.body.result),/saved/);
 assert.match(String((await send('save','t',{configurationAction:'create',name:'Destination'})).r.body.result),/saved/);
 const before=state('t').configurations.Destination;
 assert.equal((await send('start','s')).r.body.result,'accepted');
 const formerSourceActual=state('s').actual;
 // Reordered local setup JSON is the same validated prepared input, not repair.
 const targetSetup=join(dir,'t','setup.json');
 writePrivate(targetSetup,Object.fromEntries(Object.entries(JSON.parse(readFileSync(targetSetup,'utf8'))).reverse()));
 writePrivate(join(dir,'identity.json'),{secret:owner});
 const child=spawn(process.execPath,['src/cli.ts','tui',join(dir,'identity.json'),url],{stdio:['pipe','pipe','pipe']});
 let transcript='';child.stdout.on('data',x=>transcript+=x);child.stderr.on('data',x=>transcript+=x);
 const exited=new Promise(resolve=>child.once('exit',resolve));const timer=setTimeout(()=>child.kill('SIGTERM'),15000);
 try { let cursor=0;for(const [prompt,answer] of [['beehive> ','select s'],['beehive> ','move'],['Destination number: ','1'],['Confirm [yes/no]: ','yes'],['beehive> ','quit']]){await until(()=>transcript.indexOf(prompt!,cursor)>=0);cursor=transcript.indexOf(prompt!,cursor)+prompt!.length;if(answer==='quit')await until(()=>state('t').phase==='running');await delay(150);child.stdin.write(answer+'\n');}assert.equal(await exited,0); } finally {clearTimeout(timer);if(child.exitCode===null)child.kill('SIGTERM');console.log('Original reviewer actual TUI transcript:\n'+transcript);}
 assert.doesNotMatch(transcript,/preflight failed/);
 await until(()=>state('t').phase==='running');
 const move=seen.find(m=>m.type==='move'&&m.host==='s')!;
 const grant=seen.find(m=>m.type==='grant')!;
 assert.equal(state('s').assignment.assignedHost,'t'); assert.equal(state('s').phase,'stopped');
 assert.deepEqual(state('s').runs[formerSourceActual.run],formerSourceActual);
 assert.deepEqual(state('t').selected,state('t').actual.selection);
 assert.deepEqual(state('t').configurations.Destination,state('t').selected);
 assert.equal(state('t').selected.behavior.instructions,p.instructions);
 assert.equal(state('t').selected.configuration.revision,before.configuration.revision+1);
 assert.deepEqual(state('t').preparations[move.id].candidate,before);
 assert.equal(before.profile,'default'); assert.equal(before.configuration.revision,1);
 assert.equal(state('t').assignment.chain.at(-1).materialization,'named-v1');
 assert.deepEqual(state('t').assignment.chain.at(-1).selection,state('t').selected);
 const movedBytes=readFileSync(join(dir,'t','journal.json'));
 c.send(move); c.send(grant); await delay(100);
 assert.deepEqual(readFileSync(join(dir,'t','journal.json')),movedBytes);
 const actual=state('t').actual;
 const selected=await send('save','t',{configurationAction:'select',name:'Destination'});
 assert.equal(state('t').selected.profile,p.revision);assert.deepEqual(state('t').selected.configuration,actual.selection.configuration);assert.deepEqual(state('t').actual,actual);
 const bytes=readFileSync(join(dir,'t','journal.json')); c.send(selected.m);await delay(100);assert.deepEqual(readFileSync(join(dir,'t','journal.json')),bytes);
 assert.equal((await send('save','t',{configurationAction:'rename',name:'Destination',newName:'Later'},selected.m.revision)).r.body.result,'revision-conflict');
 assert.equal(String((await send('save','t',{...state('t').selected,runner:'/bin/sh'})).r.body.result).includes('Unexpected'),true);
 await send('restart','t');assert.equal(state('t').actual.selection.profile,p.revision);assert.deepEqual(state('t').actual.selection.configuration,actual.selection.configuration);assert.equal(state('t').runs[actual.run].selection.profile,p.revision);
 const restarted=state('t').actual;
 assert.deepEqual(restarted.selection,actual.selection);
 assert.deepEqual(state('t').runs[actual.run],actual);
 await s.close(); await t.close(); s=await host(join(dir,'s'),url); t=await host(join(dir,'t'),url);
 assert.equal(state('s').assignment.assignedHost,'t'); assert.equal(state('s').phase,'stopped');
 assert.deepEqual(state('s').runs[formerSourceActual.run],formerSourceActual);
 assert.deepEqual(state('t').runs[actual.run],actual);
 assert.deepEqual(state('t').runs[restarted.run],restarted);
 const reopenedBytes=readFileSync(join(dir,'t','journal.json')); c.send(grant); await delay(100);
 assert.deepEqual(readFileSync(join(dir,'t','journal.json')),reopenedBytes);
 await send('save','t',{configurationAction:'select',name:'Destination'});
 assert.equal((await send('restart','t')).r.body.result,'accepted');
 assert.deepEqual(state('t').actual.selection,actual.selection);
 console.log('PASS: original TUI Move, duplicate grant, reselect/Restart and both host reopens retain the same effective named snapshot.');

 const oldRun=state('t').actual;
 await send('save','t',{configurationAction:'select',name:'default'});
 assert.match(String((await send('save','t',{configurationAction:'remove',name:'Destination'})).r.body.result),/saved/);
 assert.deepEqual(state('t').actual,oldRun);assert.deepEqual(state('t').runs[oldRun.run],oldRun);
 const setupBefore=readFileSync(join(dir,'t','setup.json'));
 for(const body of [
   {...state('t').selected,setup:'unknown'}, {...state('t').selected,env:{TOKEN:'not-a-secret'}},
   {...state('t').selected,agent:'wrong'}, {...state('t').selected,workspace:'/not/allowed'},
   {...state('t').selected,configuration:{name:'default',revision:0}},
   {configurationAction:'select',name:'missing'}, {configurationAction:'create',name:'__proto__'},
   {configurationAction:'remove',name:'default'}, {configurationAction:'rename',name:'default',newName:'bad/name'},
 ]) {
  const before=state('t'); const r=(await send('save','t',body)).r;
  assert.doesNotMatch(String(r.body.result),/saved/);
  assert.equal(state('t').revision,before.revision);assert.deepEqual(state('t').selected,before.selected);assert.deepEqual(state('t').actual,before.actual);
 }
 assert.deepEqual(readFileSync(join(dir,'t','setup.json')),setupBefore);
 console.log('PASS: removal of inactive but actual/history-referenced configuration preserves run snapshots; 9 malformed/unknown/injection/path/name/selected-removal Saves rejected without revision/selection/actual or setup changes.');
 // Same real relay/host path with recursively reordered validated public fields.
 const reorder=(v:any):any=>Array.isArray(v)?v.map(reorder):v!==null&&typeof v==='object'?Object.fromEntries(Object.entries(v).reverse().map(([k,x])=>[k,reorder(x)])):v;
 const {behavior:_oldBehavior,...local}=state('s').selected;
 const reverse=await send('move','t',{target:'s',targetRevision:state('s').revision,selection:reorder({...local,profile:'default'})});
 assert.equal(reverse.r.body.result,'accepted'); await until(()=>state('s').phase==='running');
 assert.deepEqual(state('s').configurations.default,state('s').actual.selection);
 assert.equal(state('s').actual.selection.profile,'default');
 assert.equal(state('t').phase,'stopped');
 console.log('PASS: recursively reordered wire Move succeeds; target-local named reference and default behavior remain truthful.');
}finally{await s.close();await t.close();c.close();for(const ws of server.clients)ws.terminate();await new Promise<void>(r=>server.close(()=>r()));rmSync(dir,{recursive:true,force:true});}

});
