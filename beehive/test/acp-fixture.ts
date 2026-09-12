/** Deterministic EXTERNAL ACP fixture: never calls a provider or reads credentials. */
import { createInterface } from 'node:readline';
import { writeFileSync } from 'node:fs';
import { setTimeout as delay } from 'node:timers/promises';
const mode = process.argv[2] ?? 'ok';
let selected = '';
const send = (v: unknown) => process.stdout.write(JSON.stringify(v) + '\n');
const sessionId = 'fixture-session';
if (mode === 'teardown-drift') process.on('SIGTERM', () => send({ jsonrpc: '2.0', method: 'session/update', params: { sessionId, update: { sessionUpdate: 'current_model_update', currentModelId: 'other' } } }));
for await (const line of createInterface({ input: process.stdin })) {
  const m = JSON.parse(line);
  if (mode === 'timeout') continue;
  if (mode === 'malformed') { console.log('not-json'); continue; }
  if (mode === 'flood') { console.log('x'.repeat(1024 * 1024 + 1)); continue; }
  let result: unknown;
  if (m.method === 'initialize') result = { protocolVersion: m.params.protocolVersion, agentInfo: { name: 'buzz-agent' } };
  else if (m.method === 'session/new') result = { sessionId, models: { currentModelId: 'ignored-default', availableModels: mode === 'empty' ? [] : [{ modelId: mode === 'filtered' ? 'other-model' : ['openai','databricks-os'].includes(mode ?? '') ? process.env.BUZZ_AGENT_MODEL : 'databricks-claude-haiku-4-5', name: 'DO-NOT-RELAY-SECRET' }] } };
  else if (m.method === 'session/set_model') {
    selected = m.params.modelId;
    if (mode === 'reject') { send({ jsonrpc: '2.0', id: m.id, error: { code: -1, message: 'DO-NOT-RELAY-SECRET' } }); continue; }
    result = { sessionId, modelId: mode === 'wrong-model' ? 'other-model' : selected };
  } else if (m.method === 'session/prompt') {
    if (selected !== process.env.BUZZ_AGENT_MODEL || m.params.sessionId !== sessionId || (mode !== 'databricks-os' && process.env.DATABRICKS_TOKEN) || process.env.BUZZ_PRIVATE_KEY || process.env.BUZZ_AGENT_PROVIDER !== (mode === 'openai' ? 'openai-compat' : 'databricks_v2')) process.exit(8);
    if (mode === 'databricks-os' && (!/^http:\/\/127\.0\.0\.1:\d+$/.test(process.env.DATABRICKS_HOST ?? '') || !/^[0-9a-f]{64}$/.test(process.env.DATABRICKS_TOKEN ?? '') || process.env.BEEHIVE_DATABRICKS_RUNTIME !== undefined)) process.exit(8);
    if (mode === 'openai' && process.env.OPENAI_COMPAT_API_KEY !== 'synthetic-provider-key') process.exit(8);
    if (mode === 'delayed') { writeFileSync('prompt-started', '1'); await delay(4000); }
    const update = { jsonrpc: '2.0', method: 'session/update', params: { sessionId: mode === 'wrong-session' ? 'other-session' : sessionId, update: { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'Fixture greeting, not live model proof.' } } } };
    result = { stopReason: mode === 'cancelled' ? 'cancelled' : 'end_turn' };
    if (mode === 'bad-tail') {
      process.stdout.write(JSON.stringify(update) + '\n' + JSON.stringify({ jsonrpc: '2.0', id: m.id, result }) + '\nnot-json\n');
      continue;
    }
    const drift = { jsonrpc: '2.0', method: 'session/update', params: { sessionId: mode === 'foreign-drift' ? 'foreign-session' : sessionId, update: mode === 'config-drift' ? { sessionUpdate: 'config_option_update', configOptions: [{ category: 'model', currentValue: 'other' }] } : mode === 'non-model' ? { sessionUpdate: 'config_option_update', configOptions: [{ category: 'thought_level', currentValue: 'high' }] } : { sessionUpdate: 'current_model_update', currentModelId: 'other' } } };
    if (['current-drift', 'config-drift', 'foreign-drift', 'non-model'].includes(mode)) send(drift);
    if (mode === 'coalesced-drift') {
      process.stdout.write([update, { jsonrpc: '2.0', id: m.id, result }, drift].map(v => JSON.stringify(v) + '\n').join(''));
      continue;
    }
    send(update);
  } else process.exit(9);
  send({ jsonrpc: '2.0', id: m.id, result });
}
