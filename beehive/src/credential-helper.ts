import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import type { CredentialReference } from './credential-store.ts';

/** One owned process per read; no daemon, argv secret, journal or inherited IPC.
 * SIGKILL interrupts the process even while its JS thread is in native code.
 * Settlement waits for close (exit AND pipe closure), never just kill's return.
 * This proves process cleanup, not dismissal of an OS-owned approval dialog.
 */
export function credentialHelperReader(options: { operatorApproved: true; timeoutMs?: number; helper?: URL }) {
  const timeout = options.timeoutMs ?? 10_000;
  if (!Number.isSafeInteger(timeout) || timeout < 1 || timeout > 30_000) throw Error('Invalid credential helper deadline');
  let busy = false;
  return async (reference: CredentialReference, signal?: AbortSignal): Promise<string | null> => {
    signal?.throwIfAborted();
    if (busy) throw Error('Credential helper already busy');
    if (options.operatorApproved !== true) throw Error('Explicit owner-present credential access approval required');
    busy = true;
    try {
      return await new Promise<string | null>((resolve, reject) => {
        const child = spawn(process.execPath, [fileURLToPath(options.helper ?? new URL('./credential-helper-child.ts', import.meta.url))], {
          stdio: ['pipe', 'pipe', 'ignore'],
          // Do not inherit provider credentials, NODE_OPTIONS or loader overrides.
          env: { PATH: '/usr/bin:/bin', ...(process.env.HOME ? { HOME: process.env.HOME } : {}) },
        });
        let output = '', error: Error | undefined;
        const stop = (reason: Error) => { error ??= reason; child.kill('SIGKILL'); };
        const abort = () => stop(Error('Credential read cancelled; owned helper terminated'));
        const timer = setTimeout(() => stop(Error('Credential read timed out (possibly permission-prompt blocked); owned helper terminated; OS readiness unverified')), timeout);
        signal?.addEventListener('abort', abort, { once: true });
        child.stdout.on('data', (chunk: Buffer) => {
          if (error) return;
          if (Buffer.byteLength(output) + chunk.length > 1024) { output = ''; stop(Error('Invalid credential helper response')); return; }
          output += chunk.toString('utf8');
        });
        child.on('error', () => { error ??= Error('Credential helper unavailable'); });
        child.stdin.on('error', () => stop(Error('Credential helper pipe failed')));
        child.on('close', code => {
          clearTimeout(timer); signal?.removeEventListener('abort', abort);
          if (signal?.aborted) error ??= Error('Credential read cancelled');
          if (error) { output = ''; reject(error); return; }
          try {
            if (code !== 0) throw Error();
            const value = JSON.parse(output); output = '';
            if (value.status === 'missing') { resolve(null); return; }
            if (value.status === 'present' && typeof value.secret === 'string' && /^[0-9a-f]{64}$/.test(value.secret)) { resolve(value.secret); return; }
            throw Error();
          } catch { reject(Error('OS credential read failed (locked, denied, unavailable, or invalid response); not missing')); }
        });
        if (signal?.aborted) abort();
        else child.stdin.end(JSON.stringify(reference));
      });
    } finally { busy = false; }
  };
}
