/** Owned ACP harness fixture: exact session/model, bidirectional RPC, resistant child. */
import { createInterface } from 'node:readline';
import { readFileSync, writeFileSync } from 'node:fs';
import { spawn } from 'node:child_process';
import { publicKey } from '../src/protocol.ts';
const mode = readFileSync('mode', 'utf8');
const sessionId = 'conversation-session'; let selected = ''; let prompt: number | undefined;
writeFileSync('harness-identity', publicKey(process.env.BUZZ_PRIVATE_KEY!));
writeFileSync('harness-pid', String(process.pid));
const child = spawn(process.execPath, ['-e', "process.on('SIGTERM',()=>{}); setTimeout(()=>{},15000)"], { stdio: 'ignore' });
writeFileSync('descendant-pid', String(child.pid));
process.on('SIGTERM', () => {});
setTimeout(() => process.exit(0), 15000).unref();
const send = (m: any) => process.stdout.write(JSON.stringify(m) + '\n');
for await (const line of createInterface({ input: process.stdin })) {
  const m = JSON.parse(line); let result: any;
  if (!m.method) { if (m.result?.ok) writeFileSync('reverse-rpc', 'ok'); continue; }
  if (m.method === 'initialize') result = { protocolVersion: 1, agentInfo: { name: 'buzz-agent' } };
  else if (m.method === 'session/new') result = { sessionId, models: { currentModelId: 'default', availableModels: [] } };
  else if (m.method === 'session/load') { selected = ''; result = {}; }
  else if (m.method === 'session/set_model') {
    if (mode === 'reject') { send({ jsonrpc: '2.0', id: m.id, error: { code: -1, message: 'secret-diagnostic' } }); continue; }
    selected = m.params.modelId; result = { sessionId, modelId: mode === 'wrong-model' ? 'other' : selected };
  } else if (m.method === 'session/cancel') {
    writeFileSync('cancel-observed', 'yes');
    if (prompt) send({ jsonrpc: '2.0', id: prompt, result: { stopReason: 'cancelled' } }); continue;
  } else if (m.method === 'session/prompt') {
    if (selected !== process.env.BUZZ_AGENT_MODEL || m.params.sessionId !== sessionId) process.exit(8);
    writeFileSync('prompt-started', selected); prompt = m.id;
    send({ jsonrpc: '2.0', id: 'reverse', method: 'client/fixture', params: {} });
    if (mode === 'delayed' || mode === 'cancel') continue;
    const update = { jsonrpc: '2.0', method: 'session/update', params: { sessionId, update: { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'Private conversation fixture response' } } } };
    const reply = { jsonrpc: '2.0', id: m.id, result: { stopReason: 'end_turn' } };
    process.stdout.write(JSON.stringify(update) + '\n' + JSON.stringify(reply) + '\n' + (mode === 'bad-tail' ? 'not-json\n' : '')); continue;
  } else throw Error('Unexpected method');
  send({ jsonrpc: '2.0', id: m.id, result });
}
