import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { databricksRuntime } from './databricks-runtime.ts';
import { piModelConfig } from './pi.ts';

// Entire wrapper, adapter, Pi children and native helpers are in the existing
// supervisor group. No provider secret is persisted, including on forced exit.
let stopping = false;
let bridge: Awaited<ReturnType<typeof databricksRuntime>> | undefined;
let child: ReturnType<typeof spawn> | undefined;
let directory: string | undefined;
const stop = async () => { stopping = true; child?.kill('SIGTERM'); await bridge?.stop(); };
process.on('SIGTERM', () => { void stop(); });
process.on('SIGINT', () => { void stop(); });
try {
  const { provider, model } = JSON.parse(process.env.BEEHIVE_PI_RUNTIME ?? '');
  delete process.env.BEEHIVE_PI_RUNTIME;
  if (provider.provider === 'databricks_v2') bridge = await databricksRuntime({ host: provider.baseUrl, key: provider.credential, model });
  if (!stopping) {
    directory = mkdtempSync(join(tmpdir(), 'beehive-pi-'));
    const agent = join(directory, 'agent'); mkdirSync(agent, { mode: 0o700 });
    writeFileSync(join(agent, 'models.json'), JSON.stringify(piModelConfig(provider, model, bridge?.endpoint)), { mode: 0o600 });
    const executable = process.argv[2]; if (!executable) throw Error();
    child = spawn(executable, ['--', '--provider', 'beehive', '--model', model, '--thinking', provider.effort ?? 'off', '--no-extensions', '--no-skills', '--no-prompt-templates', '--no-themes'], { stdio: ['inherit','inherit','ignore'], env: { ...process.env, HOME: directory, PI_CODING_AGENT_DIR: agent, BEEHIVE_PI_KEY: bridge?.capability ?? process.env.BEEHIVE_PI_KEY! } });
    await new Promise<void>((resolve,reject) => { child!.once('error',reject); child!.once('close', code => { process.exitCode = stopping ? 0 : code ?? 1; resolve(); }); });
  }
} catch { process.exitCode = 1; }
finally { await stop(); if (directory) rmSync(directory, { recursive: true, force: true }); }
