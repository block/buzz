/** Deterministic EXTERNAL ACP fixture: never calls a provider or reads credentials. */
import { createInterface } from 'node:readline';
const mode = process.argv[2] ?? 'ok';
let selected = '';
const send = (v: unknown) => process.stdout.write(JSON.stringify(v) + '\n');
const sessionId = 'fixture-session';
for await (const line of createInterface({ input: process.stdin })) {
  const m = JSON.parse(line);
  if (mode === 'timeout') continue;
  if (mode === 'malformed') { console.log('not-json'); continue; }
  if (mode === 'flood') { console.log('x'.repeat(1024 * 1024 + 1)); continue; }
  let result: unknown;
  if (m.method === 'initialize') result = { protocolVersion: 1, agentInfo: { name: 'buzz-agent' } };
  else if (m.method === 'session/new') result = { sessionId, models: { currentModelId: 'ignored-default', availableModels: mode === 'empty' ? [] : [{ modelId: mode === 'filtered' ? 'other-model' : 'databricks-claude-haiku-4-5', name: 'DO-NOT-RELAY-SECRET' }] } };
  else if (m.method === 'session/set_model') {
    selected = m.params.modelId;
    if (mode === 'reject') { send({ jsonrpc: '2.0', id: m.id, error: { code: -1, message: 'DO-NOT-RELAY-SECRET' } }); continue; }
    result = { sessionId, modelId: mode === 'wrong-model' ? 'other-model' : selected };
  } else if (m.method === 'session/prompt') {
    if (selected !== process.env.BUZZ_AGENT_MODEL || m.params.sessionId !== sessionId || process.env.DATABRICKS_TOKEN || process.env.BUZZ_PRIVATE_KEY || process.env.BUZZ_AGENT_PROVIDER !== 'databricks_v2') process.exit(8);
    send({ jsonrpc: '2.0', method: 'session/update', params: { sessionId: mode === 'wrong-session' ? 'other-session' : sessionId, update: { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'Fixture greeting, not live model proof.' } } } });
    result = { stopReason: mode === 'cancelled' ? 'cancelled' : 'end_turn' };
  } else process.exit(9);
  send({ jsonrpc: '2.0', id: m.id, result });
}
