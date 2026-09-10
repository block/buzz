import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';

/** In-lifetime group ownership: a live TS anchor retains the group ID until its own Stop. */
export function spawnOwned(executable: string, args: readonly string[], cwd: string, env: NodeJS.ProcessEnv) {
  const child = spawn(process.execPath, [fileURLToPath(new URL('./supervisor.ts', import.meta.url))], {
    cwd, env: { PATH: '/usr/bin:/bin' }, detached: true, stdio: ['pipe', 'pipe', 'pipe', 'ipc'],
  }) as ChildProcessWithoutNullStreams;
  let runnerExited = false;
  let spawned = false;
  let rejectReady: (e: Error) => void = () => {};
  const ready = new Promise<void>((resolve, reject) => {
    rejectReady = reject;
    child.once('error', () => reject(Error('Group supervisor unavailable')));
    child.once('exit', () => { runnerExited = true; reject(Error('Group supervisor exited')); });
    child.on('message', (m: unknown) => {
      const type = (m as { type?: string }).type;
      if (type === 'spawned') { spawned = true; resolve(); }
      if (type === 'runner-error' || type === 'runner-exit') {
        runnerExited = true;
        if (!spawned) reject(Error('External harness failed to launch'));
      }
    });
    child.once('spawn', () => child.send({ type: 'launch', executable, args: [...args], cwd, env }, error => { if (error) reject(Error('Group launch channel failed')); }));
  });
  // Readiness deadline bounds even a supervisor that never acknowledges launch.
  const timer = setTimeout(() => rejectReady(Error('Group launch timed out')), 5000);
  void ready.then(() => clearTimeout(timer), () => clearTimeout(timer));
  return {
    child, ready,
    get exited() { return runnerExited; },
    async stop() {
      const pid = child.pid;
      if (!pid || !child.connected) throw Error('Live group anchor unavailable; reconcile locally');
      await new Promise<void>((resolve, reject) => child.send({ type: 'stop' }, error => error ? reject(Error('Owned Stop channel failed')) : resolve()));
      for (let i = 0; i < 100; i++) {
        try { process.kill(-pid, 0); }
        catch (e) { if ((e as NodeJS.ErrnoException).code === 'ESRCH') return; throw e; }
        await delay(25);
      }
      throw Error('Owned process group did not exit; quarantined');
    },
  };
}
/** Only the returned in-memory handle authorizes Stop; no reconstruction from journal PIDs. */
export type OwnedProcess = ReturnType<typeof spawnOwned>;
