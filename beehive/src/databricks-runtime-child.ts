import { spawn } from 'node:child_process';
import { databricksRuntime } from './databricks-runtime.ts';

// Owned by the existing POSIX supervisor group. No independent daemon, restart,
// login, token files or PID adoption. Only ACP bytes are written to stdout.
let stopping = false;
let bridge: Awaited<ReturnType<typeof databricksRuntime>> | undefined;
let child: ReturnType<typeof spawn> | undefined;
const stop = async () => { stopping = true; child?.kill('SIGTERM'); await bridge?.stop(); };
process.on('SIGTERM', () => { void stop(); });
process.on('SIGINT', () => { void stop(); });
try {
  const config = JSON.parse(process.env.BEEHIVE_DATABRICKS_RUNTIME ?? '');
  delete process.env.BEEHIVE_DATABRICKS_RUNTIME;
  bridge = await databricksRuntime(config);
  if (stopping) await bridge.stop();
  else {
    const executable = process.argv[2]; if (!executable) throw Error();
    child = spawn(executable, process.argv.slice(3), { stdio: ['inherit', 'inherit', 'ignore'], env: { ...process.env, DATABRICKS_HOST: bridge.endpoint, DATABRICKS_TOKEN: bridge.capability } });
    await new Promise<void>((resolve, reject) => { child!.once('error', reject); child!.once('close', code => { process.exitCode = stopping ? 0 : code ?? 1; resolve(); }); });
    await bridge.stop();
  }
} catch { await stop(); process.exitCode = 1; }
