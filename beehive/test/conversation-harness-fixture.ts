/** Owned ACP harness fixture: exact session/model, bidirectional RPC, resistant child. */
import { createInterface } from 'node:readline';
import { readFileSync, writeFileSync } from 'node:fs';
import { spawn } from 'node:child_process';
import { publicKey } from '../src/protocol.ts';
const mode = readFileSync('mode', 'utf8');
let tool: any; let configRejected = false;
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
  else if (m.method === 'session/new') { tool = m.params.mcpServers[0]; result = { sessionId, models: { currentModelId: 'default', availableModels: [] } }; }
  else if (m.method === 'session/load') { selected = ''; result = {}; }
  else if (m.method === 'session/set_model') {
    if (mode === 'reject' || (mode === 'optional-reack-fails' && configRejected)) { send({ jsonrpc: '2.0', id: m.id, error: { code: -1, message: 'secret-diagnostic' } }); continue; }
    selected = m.params.modelId; result = { sessionId, modelId: mode === 'wrong-model' ? 'other' : selected };
  } else if (m.method === 'session/set_config_option') {
    if (mode !== 'optional-ok') {
      configRejected = true;
      send({ jsonrpc: '2.0', id: m.id, error: { code: -32601, message: 'unsupported optional setting' } }); continue;
    }
    result = {};
  } else if (m.method === 'session/cancel') {
    writeFileSync('cancel-observed', 'yes');
    if (prompt) send({ jsonrpc: '2.0', id: prompt, result: { stopReason: 'cancelled' } }); continue;
  } else if (m.method === 'session/prompt') {
    if (selected !== process.env.BUZZ_AGENT_MODEL || m.params.sessionId !== sessionId) process.exit(8);
    writeFileSync('prompt-started', selected); prompt = m.id;
    send({ jsonrpc: '2.0', id: 'reverse', method: 'client/fixture', params: {} });
    if (mode === 'delayed' || mode === 'cancel') continue;
    if (tool) {
      // Like buzz-agent's MCP transport, only the shim deliberately escapes.
      const mcp = spawn(tool.command, tool.args, { detached: true, stdio: ['pipe', 'pipe', 'pipe'], env: { PATH: '/usr/bin:/bin' } });
      writeFileSync('tool-shim-pid', String(mcp.pid)); mcp.stderr.resume();
      const lines = createInterface({ input: mcp.stdout });
      const pending = new Map<number, (m: any) => void>();
      lines.on('line', line => { const r = JSON.parse(line); pending.get(r.id)?.(r); });
      let seq = 0;
      const rpc = (method: string, params: any) => new Promise<any>((resolve, reject) => {
        const id = ++seq; const timer = setTimeout(() => reject(Error('MCP fixture timeout')), 8000);
        pending.set(id, m => { clearTimeout(timer); pending.delete(id); m.error || m.result?.isError ? reject(Error('MCP fixture failed')) : resolve(m.result); });
        mcp.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
      });
      await rpc('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'fixture', version: '1' } });
      mcp.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n');
      const tools = await rpc('tools/list', {});
      if (tools.tools[0].name !== 'buzz_reply') throw Error('Missing scoped reply tool');
      await rpc('tools/call', { name: 'buzz_reply', arguments: { content: 'Private conversation fixture response' } });
      // Remain connected until host Stop; the MCP shim has no children.
    }
    const update = { jsonrpc: '2.0', method: 'session/update', params: { sessionId, update: { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'Private conversation fixture response' } } } };
    const reply = { jsonrpc: '2.0', id: m.id, result: { stopReason: 'end_turn' } };
    process.stdout.write(JSON.stringify(update) + '\n' + JSON.stringify(reply) + '\n' + (mode === 'bad-tail' ? 'not-json\n' : '')); continue;
  } else throw Error('Unexpected method');
  send({ jsonrpc: '2.0', id: m.id, result });
}
