import { databricksNative } from './databricks.ts';
import { fork } from 'node:child_process';
import { fileURLToPath } from 'node:url';

/** One bounded Node helper per explicit credential operation. Completion waits for exit.
 * Cancellation cannot undo OS persistence; callers must inspect before retrying. */
export function managerCredential(input: object, signal: AbortSignal, helper = new URL('./manager-credential-child.ts', import.meta.url)): Promise<any> {
  signal.throwIfAborted();
  const request = input as any;
  if (request.action === 'provider-read' && request.provider === 'databricks_v2') return databricksNative({ action:'token',host:request.host,key:request.key },signal);
  return new Promise((resolve, reject) => {
    const child = fork(fileURLToPath(helper), [], {
      execPath: process.execPath, execArgv: [], silent: true,
      env: { PATH: '/usr/bin:/bin', ...(process.env.HOME ? { HOME: process.env.HOME } : {}) },
    });
    let result: any; let failed = false;
    const cancel = () => { failed = true; child.kill('SIGKILL'); };
    const timer = setTimeout(cancel, 10000);
    signal.addEventListener('abort', cancel, { once: true });
    child.stdout?.resume(); child.stderr?.resume();
    child.once('error', () => { failed = true; });
    child.once('message', value => { result = value; });
    child.once('close', code => {
      clearTimeout(timer); signal.removeEventListener('abort', cancel);
      if (code !== 0 || failed || signal.aborted || !result?.ok) reject(Error('Could not complete key access. It may have been cancelled, timed out, denied, or unavailable. The key may not match. Changes may already be saved. Inspect before you try again. No key was reset or saved as plain text.'));
      else resolve(result);
    });
    child.send(input, error => { if (error) cancel(); });
  });
}
