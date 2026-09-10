/** Internal POSIX group anchor. Never invoke directly; the parent provisions one launch via IPC. */
import { spawn } from 'node:child_process';

if (!process.send) throw Error('Supervisor requires a private IPC channel');
let launched = false;
const keepalive = setInterval(() => {}, 1000);
process.on('message', (message: unknown) => {
  const m = message as { type?: string; executable?: string; args?: string[]; cwd?: string; env?: NodeJS.ProcessEnv };
  if (m.type === 'stop') {
    // The group leader signals ITS OWN group: never a recovered or reused numeric identity.
    process.kill(-process.pid, 'SIGTERM');
  } else if (m.type === 'launch' && !launched && m.executable) {
    launched = true;
    const runner = spawn(m.executable, m.args ?? [], { cwd: m.cwd, env: m.env, stdio: ['inherit', 'inherit', 'inherit'] });
    runner.once('spawn', () => process.send?.({ type: 'spawned' }));
    runner.once('error', () => process.send?.({ type: 'runner-error' }));
    runner.once('exit', () => process.send?.({ type: 'runner-exit' }));
  }
});
// Parent loss tears down the known group, without interpreting a persisted PID.
process.once('disconnect', () => { clearInterval(keepalive); process.kill(-process.pid, 'SIGTERM'); });
