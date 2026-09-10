/** External runtime contract fixture, NOT a replacement Buzz relay implementation. */
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { writeFileSync, readFileSync } from 'node:fs';
import { publicKey } from '../src/protocol.ts';
const mode = readFileSync('mode', 'utf8');
writeFileSync('runtime-identity', publicKey(process.env.BUZZ_PRIVATE_KEY!));
writeFileSync('runtime-pid', String(process.pid));
const child = spawn(process.env.BUZZ_ACP_AGENT_COMMAND!, process.env.BUZZ_ACP_AGENT_ARGS!.split(','), {
  // Reproduce AcpClient::spawn's process_group(0) escape, not naive inheritance.
  detached: true, stdio: ['pipe', 'pipe', 'ignore'], env: { PATH: '/usr/bin:/bin' },
});
writeFileSync('shim-pid', String(child.pid));
setTimeout(() => process.exit(0), 15000).unref();
const pending = new Map<number, (m: any) => void>(); let seq = 0;
const input = createInterface({ input: child.stdout });
input.on('line', line => { const m = JSON.parse(line); if (m.method === 'client/fixture') child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: m.id, result: { ok: true } }) + '\n'); else pending.get(m.id)?.(m); });
child.on('exit', () => process.exit(0));
async function request(method: string, params: unknown) {
  const id = ++seq;
  return new Promise<any>(resolve => { pending.set(id, resolve); child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n'); });
}
await request('initialize', { protocolVersion: 1, clientCapabilities: {} });
const created = await request('session/new', { cwd: mode === 'foreign-workspace' ? '/' : process.cwd(), mcpServers: mode === 'injected-mcp' ? [{ name: 'bad', command: '/bin/sh', args: ['-c', 'exit 0'], env: [] }] : [] });
const sessionId = created.result.sessionId;
if (mode === 'conflict') await request('session/set_model', { sessionId, modelId: 'wrong' });
else await request('session/set_model', { sessionId, modelId: process.env.BUZZ_ACP_MODEL });
await request('session/load', { sessionId, cwd: process.cwd(), mcpServers: [] });
if (['optional-ok', 'optional-reject', 'optional-reack-fails', 'model-config-reject'].includes(mode)) {
  const config = await request('session/set_config_option', { sessionId, configId: mode === 'model-config-reject' ? 'model' : 'mode', value: mode === 'model-config-reject' ? process.env.BUZZ_ACP_MODEL : 'default' });
  if (config.error) writeFileSync('config-error-forwarded', 'yes');
}
const prompt = request('session/prompt', { sessionId, prompt: [{ type: 'text', text: 'External runtime conversation input' }] });
if (mode === 'cancel') setTimeout(() => child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'session/cancel', params: { sessionId } }) + '\n'), 50);
const response = await prompt;
writeFileSync('runtime-completed', response.result.stopReason);
