/** Owned ACP harness fixture: exact session/model, bidirectional RPC, resistant child. */
import { createInterface } from 'node:readline';
import { readFileSync, writeFileSync, appendFileSync, renameSync } from 'node:fs';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { publicKey } from '../src/protocol.ts';
if (process.env.BUZZ_AGENT_SYSTEM_PROMPT) writeFileSync('received-system-instructions', process.env.BUZZ_AGENT_SYSTEM_PROMPT);
const codex = process.env.CODEX_CONFIG !== undefined;
if (codex) {
  const config = JSON.parse(process.env.CODEX_CONFIG!);
  if (Object.keys(config).join() !== 'model' || process.argv.length !== 2 || process.env.GOOSE_PROVIDER || process.env.ANTHROPIC_MODEL || process.env.BUZZ_AGENT_CONFIG_DIR || process.env.BUZZ_ACP_MODEL || process.env.OPENAI_API_KEY !== 'fixture-codex-private-key' || !process.env.CODEX_HOME?.endsWith('agent-config') || !process.env.HOME?.endsWith('service-home') || process.env.CODEX_HOME === process.env.HOME) throw Error('Incorrect Codex config/auth/environment');
  writeFileSync('codex-env-checked', 'explicit key + separate CODEX_HOME + JSON model override');
}
const claude = process.env.ANTHROPIC_MODEL !== undefined;
if (claude) {
  if (process.argv.length !== 2 || process.env.GOOSE_PROVIDER || process.env.BUZZ_AGENT_CONFIG_DIR || process.env.BUZZ_ACP_MODEL || process.env.ANTHROPIC_API_KEY !== 'fixture-claude-private-key' || !process.env.CLAUDE_CODE_EXECUTABLE || !process.env.HOME?.endsWith('service-home')) throw Error('Incorrect Claude private environment/argv');
  writeFileSync('claude-env-checked', 'private key + CLI + dedicated HOME, no mixed provider vars');
}
const goose = process.env.GOOSE_PROVIDER !== undefined;
if (goose && (process.env.BUZZ_AGENT_MODEL || process.env.BUZZ_AGENT_CONFIG_DIR || process.env.GOOSE_MODE !== 'auto')) throw Error('Mixed harness environment');
const model = codex ? JSON.parse(process.env.CODEX_CONFIG!).model : claude ? process.env.ANTHROPIC_MODEL : goose ? process.env.GOOSE_MODEL : process.env.BUZZ_AGENT_MODEL;
const mode = readFileSync('mode', 'utf8');
const nativeModel = () => ({ configOptions: [{ configId: 'goose-model', category: 'model', currentValue: mode === 'wrong-model' ? 'other' : model, options: [{ value: model }] }] });
let tool: any; let configRejected = false;
const gooseSessions = new Set<string>(); let sessionNumber = 0; let sessionId = 'conversation-session'; let selected = ''; let prompt: number | undefined;
if (process.env.BUZZ_PRIVATE_KEY) writeFileSync('harness-identity', publicKey(process.env.BUZZ_PRIVATE_KEY));
else writeFileSync('preflight-no-identity', 'yes');
writeFileSync('harness-pid', String(process.pid));
const child = spawn(process.execPath, ['-e', "process.on('SIGTERM',()=>{}); setTimeout(()=>{},60000)"], { stdio: 'ignore' });
await once(child, 'spawn');
if (!Number.isSafeInteger(child.pid) || child.pid! <= 0) throw Error('Invalid fixture child PID');
writeFileSync('descendant-pid', String(child.pid));
process.on('SIGTERM', () => {});
setTimeout(() => process.exit(0), 60000).unref();
const send = (m: any) => process.stdout.write(JSON.stringify(m) + '\n');
for await (const line of createInterface({ input: process.stdin })) {
  const m = JSON.parse(line); let result: any;
  if (codex) appendFileSync('codex-rpc-methods', JSON.stringify({method:m.method}) + '\n');
  if (claude) appendFileSync('claude-rpc-methods', JSON.stringify({method:m.method}) + '\n');
  if (goose) appendFileSync('goose-rpc-methods', JSON.stringify({method:m.method, configId:m.params?.configId, value:m.params?.value}) + '\n');
  if (!m.method) {
    if (m.result?.ok) {
      // Readers wait for existence: publish complete bytes, not open/truncate before write.
      const temporary = `reverse-rpc.${process.pid}.tmp`;
      writeFileSync(temporary, 'ok'); renameSync(temporary, 'reverse-rpc');
    }
    continue;
  }
  if (codex && m.method === 'initialize') {
    if (m.params.protocolVersion !== 2) throw Error('Codex requires requested protocol 2');
    result = { protocolVersion: mode === 'wrong-protocol' ? 1 : 2, agentInfo: { name: mode === 'missing-native' ? 'unknown' : 'codex-acp' } };
  }
  else if (codex && m.method === 'session/new') {
    if (mode === 'auth-rejected') { send({ jsonrpc: '2.0', id: m.id, error: { code: -32000, message: 'fixture auth rejected' } }); continue; }
    if (m.params._meta?.systemPrompt !== undefined) throw Error('Codex never accepts Claude append');
    if (m.params.systemPrompt !== undefined) writeFileSync('received-system-instructions', m.params.systemPrompt);
    sessionId = `codex-${process.pid}-${++sessionNumber}`; gooseSessions.add(sessionId);
    tool = m.params.mcpServers[0]; selected = model;
    result = { sessionId, ...(mode === 'missing-model' ? {} : { models: { currentModelId: mode === 'wrong-model' ? 'other' : model, availableModels: [{ modelId: model }] } }) };
  }
  else if (m.method === 'initialize') result = { protocolVersion: 1, agentInfo: { name: claude ? (mode === 'missing-native' ? 'claude-code-acp' : '@agentclientprotocol/claude-agent-acp') : goose ? 'goose' : 'buzz-agent' } };
  else if (m.method === 'session/new' && claude) {
    if (mode === 'auth-rejected') { send({ jsonrpc: '2.0', id: m.id, error: { code: -32000, message: 'fixture auth rejected' } }); continue; }
    sessionId = `claude-${process.pid}-${++sessionNumber}`; gooseSessions.add(sessionId);
    tool = m.params.mcpServers[0]; selected = model!;
    if (m.params.systemPrompt !== undefined) throw Error('Claude must use native meta append');
    if (m.params._meta?.systemPrompt) writeFileSync('received-system-instructions', m.params._meta.systemPrompt.append);
    result = { sessionId, models: { currentModelId: mode === 'wrong-model' ? 'other' : model, availableModels: [{ modelId: model }] } };
  }
  else if (m.method === 'session/new') { if (goose) sessionId = `goose-${process.pid}-${++sessionNumber}`; if (goose) gooseSessions.add(sessionId); tool = m.params.mcpServers[0]; selected = goose ? model! : ''; result = goose ? { sessionId, ...nativeModel() } : { sessionId, models: { currentModelId: 'default', availableModels: [] } }; }
  else if (goose && m.method === '_goose/unstable/session/system-prompt/set') {
    if (m.params.sessionId !== sessionId || m.params.mode !== 'set' || m.params.key !== 'buzz' || typeof m.params.text !== 'string') throw Error('Invalid Goose prompt request');
    writeFileSync('received-system-instructions', m.params.text); result = {};
  }
  else if (m.method === 'session/load') { selected = ''; result = {}; }
  else if (m.method === 'session/set_model') {
    if (codex) throw Error('Codex must not receive model/config switch');
    if (claude) throw Error('Claude never accepts invented set_model acknowledgement');
    if (goose) throw Error('Goose must never receive unstable set_model');
    if (mode === 'reject' || (mode === 'optional-reack-fails' && configRejected)) { send({ jsonrpc: '2.0', id: m.id, error: { code: -1, message: 'secret-diagnostic' } }); continue; }
    selected = m.params.modelId; result = { sessionId, modelId: mode === 'wrong-model' ? 'other' : selected };
  } else if (m.method === 'session/set_config_option') {
    if (codex) throw Error('Codex must not receive model/config switch');
    if (claude) throw Error('Claude native config unsupported');
    if (goose) { if (m.params.configId !== 'goose-model' || m.params.value !== model) throw Error('Unsupported Goose config'); result = nativeModel(); }
    else if (mode !== 'optional-ok') {
      configRejected = true;
      send({ jsonrpc: '2.0', id: m.id, error: { code: -32601, message: 'unsupported optional setting' } }); continue;
    }
    else result = {};
  } else if (m.method === 'session/cancel') {
    writeFileSync('cancel-observed', 'yes');
    if (prompt) send({ jsonrpc: '2.0', id: prompt, result: { stopReason: 'cancelled' } }); continue;
  } else if (m.method === 'session/prompt') {
    if ((goose || claude || codex) && gooseSessions.has(m.params.sessionId)) sessionId = m.params.sessionId;
    appendFileSync('received-prompts.jsonl', JSON.stringify(m.params) + '\n');
    if (selected !== model || m.params.sessionId !== sessionId) process.exit(8);
    writeFileSync('prompt-started', selected); prompt = m.id;
    if ((claude && mode === 'claude-drift') || (codex && mode === 'codex-drift')) send({ jsonrpc: '2.0', method: 'session/update', params: { sessionId, update: { sessionUpdate: 'current_model_update', currentModelId: 'wrong' } } });
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
      if (tools.tools[0].name !== 'buzz') throw Error('Missing Buzz CLI tool');
      // Fixture model interprets upstream instructions, NOT host permission logic.
      const text = m.params.prompt.map((p: any) => p.text ?? '').join('\n');
      writeFileSync('last-prompt', text);
      const parent = [...text.matchAll(/--reply-to ([a-f0-9]{64})/g)].at(-1)?.[1];
      const channel = [...text.matchAll(/[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}/g)].at(-1)?.[0];
      const recipient = readFileSync('recipient', 'utf8');
      if (!parent || !channel) throw Error('Fixture model could not interpret routing');
      const read = await rpc('tools/call', { name: 'buzz', arguments: { argv: ['channels', 'list'] } });
      if (!JSON.stringify(read).includes(channel)) throw Error('CLI read did not return channel');
      let rejected = false;
      try { await rpc('tools/call', { name: 'buzz', arguments: { argv: ['messages', 'send', '--channel', channel, '--reply-to', parent, '--mention', readFileSync('outsider', 'utf8'), '--content', 'must not publish'] } }); }
      catch { rejected = true; }
      if (!rejected) throw Error('CLI accepted non-member mention');
      appendFileSync('rejected-mentions', parent + '\n');
      await rpc('tools/call', { name: 'buzz', arguments: { argv: ['messages', 'send', '--channel', channel, '--reply-to', parent, '--mention', recipient, '--content', '-'], stdin: 'Private conversation fixture response' } });
      appendFileSync('completed-tools', parent + '\n');
      // Remain connected until host Stop; the MCP shim has no children.
    }
    const update = { jsonrpc: '2.0', method: 'session/update', params: { sessionId, update: { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'Private conversation fixture response' } } } };
    const reply = { jsonrpc: '2.0', id: m.id, result: { stopReason: 'end_turn' } };
    process.stdout.write(JSON.stringify(update) + '\n' + JSON.stringify(reply) + '\n' + (mode === 'bad-tail' ? 'not-json\n' : '')); continue;
  } else throw Error('Unexpected method');
  send({ jsonrpc: '2.0', id: m.id, result });
}
