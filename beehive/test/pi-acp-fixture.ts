import { createInterface } from 'node:readline';
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { spawn } from 'node:child_process';
import assert from 'node:assert/strict';
const config = JSON.parse(readFileSync(join(process.cwd(),'pi-fixture.json'),'utf8'));
const agent = process.env.PI_CODING_AGENT_DIR!;
const document = readFileSync(join(agent,'models.json'),'utf8');
const provider = JSON.parse(document).providers.beehive;
assert.equal(provider.apiKey,'$BEEHIVE_PI_KEY');
assert.equal(provider.models[0].id,config.model);
assert.equal(provider.models[0].name,config.model);
assert.equal(document.includes('synthetic'),false);
const args = process.argv.slice(2);
assert.equal(args[args.indexOf('--model')+1],config.model);
assert.equal(args[args.indexOf('--thinking')+1],config.effort ?? 'off');
assert.equal(process.env.PI_ACP_PI_COMMAND,process.execPath);
assert.equal(process.env.OPENAI_API_KEY,undefined);
assert.equal(process.env.DATABRICKS_TOKEN,undefined);
assert.equal(process.env.BEEHIVE_PI_RUNTIME,undefined);
const descendant = spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{stdio:'ignore'});
writeFileSync(join(process.cwd(),'pi-owned.json'),JSON.stringify({adapter:process.pid,descendant:descendant.pid,directory:process.env.HOME,endpoint:provider.baseUrl}));
const sessionId = 'pi-fixture-session';
const configOptions = [{id:'model',category:'model',currentValue:`beehive/${config.model}`,options:[{value:`beehive/${config.model}`} ]},{id:'thought_level',category:'thought_level',currentValue:config.effort ?? 'off',options:[{value:config.effort ?? 'off'}]}];
const send = (m: unknown) => console.log(JSON.stringify({jsonrpc:'2.0',...m as object}));
createInterface({input:process.stdin}).on('line',async line => {
 const m=JSON.parse(line); let result;
 if(m.method==='initialize') result={protocolVersion:config.mode==='wrong-protocol'?2:1,agentInfo:{name:config.mode==='wrong-fork'?'pi-acp':'buzz-pi-acp',version:'0.0.33'}};
 else if(m.method==='session/new') { if(config.instructions) assert.equal(m.params._meta?.systemPrompt,config.instructions); if(config.mode==='wrong-model') configOptions[0].currentValue='beehive/wrong'; if(config.mode==='wrong-effort') configOptions[1].currentValue='low'; result={sessionId,configOptions}; }
 else if(m.method==='session/prompt') {
   assert.equal(m.params.sessionId,sessionId);
   if(config.mode==='drift-effort' || config.mode==='drift-model') { send({method:'session/update',params:{sessionId,update:config.mode==='drift-effort'?{sessionUpdate:'current_mode_update',currentModeId:'low'}:{sessionUpdate:'current_model_update',currentModelId:'beehive/wrong'}}});return; }
   for(let n=0;n<(config.db?2:1);n++) {
     const endpoint=config.db?provider.baseUrl+'/responses':config.endpoint+'/v1/responses';
     if(!config.db) assert.equal(provider.baseUrl,'https://api.openai.com/v1');
     const response=await fetch(endpoint,{method:'POST',headers:{authorization:'Bearer '+process.env[provider.apiKey.slice(1)]},body:JSON.stringify({model:provider.models[0].id, ...(config.effort?{reasoning:{effort:args[args.indexOf('--thinking')+1]}}:{}),input:'synthetic'})});
     assert.equal(response.status,200); await response.text();
     if(config.db) await new Promise(r=>setTimeout(r,75));
   }
   send({method:'session/update',params:{sessionId,update:{sessionUpdate:'agent_message_chunk',content:{type:'text',text:'Synthetic Pi complete'}}}}); result={stopReason:'end_turn'};
 } else if(m.method==='session/cancel') return;
 else throw Error('Unexpected Pi RPC '+m.method);
 send({id:m.id,result});
});
