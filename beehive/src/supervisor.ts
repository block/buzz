/** Internal POSIX group anchor. Never invoke directly; the parent provisions one launch via IPC. */
import { spawn } from 'node:child_process';

if (!process.send) throw Error('Supervisor requires a private IPC channel');
let launched = false;
const keepalive = setInterval(() => {}, 1000);
let stopping = false;
// Retain group identity through graceful shutdown, even if a runner ignores TERM.
process.on('SIGTERM', () => {});
function stopGroup() {
  if (stopping) return;
  stopping = true;
  process.kill(-process.pid, 'SIGTERM');
  // Escalation comes from the STILL LIVING group leader, never a recovered PGID.
  setTimeout(() => process.kill(-process.pid, 'SIGKILL'), 500);
}
function notify(type: string) {
  if (!process.connected) { stopGroup(); return; }
  process.send?.({ type }, (error: Error | null) => { if (error) stopGroup(); });
}
process.on('message', (message: unknown) => {
  const m = message as { type?: string; executable?: string; args?: string[]; cwd?: string; env?: NodeJS.ProcessEnv };
  if (m.type === 'stop') {
    // The group leader signals ITS OWN group: never a recovered or reused numeric identity.
    stopGroup();
  } else if (m.type === 'launch' && !launched && m.executable) {
    launched = true;
    const runner = spawn(m.executable, m.args ?? [], { cwd: m.cwd, env: m.env, stdio: ['inherit', 'inherit', 'inherit'] });
    runner.once('spawn', () => notify('spawned'));
    runner.once('error', () => notify('runner-error'));
    runner.once('exit', () => notify('runner-exit'));
  }
});
// Parent loss tears down the known group, without interpreting a persisted PID.
process.once('disconnect', () => { clearInterval(keepalive); stopGroup(); });
