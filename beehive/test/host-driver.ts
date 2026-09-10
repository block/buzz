import { type ChildProcess } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';

/** Observe an owned fixture host without discarding startup failure evidence.
 * Only use with fresh fixture identities: bounded child output is diagnostic data.
 */
export async function hostReady(child: ChildProcess, label: string): Promise<void> {
  const limit = 8192;
  let stdout = Buffer.alloc(0), stderr = Buffer.alloc(0);
  let failure: Error | undefined;
  const retain = (previous: Buffer, chunk: Buffer) => Buffer.concat([previous, chunk]).subarray(-limit);
  const out = (chunk: Buffer) => { stdout = retain(stdout, chunk); };
  const err = (chunk: Buffer) => { stderr = retain(stderr, chunk); };
  const error = (value: Error) => { failure = value; };
  child.stdout?.on('data', out); child.stderr?.on('data', err); child.on('error', error);
  try {
    for (let i = 0; i < 400; i++) {
      if (failure || child.exitCode !== null || child.signalCode !== null) break;
      if (stdout.includes('Host online')) return;
      await delay(20);
    }
    throw Error(`Host startup/readiness failed (${label}); pid=${child.pid ?? 'unspawned'} exit=${child.exitCode} signal=${child.signalCode} error=${failure?.message ?? 'none'}; stdout tail=${JSON.stringify(stdout.toString())}; stderr tail=${JSON.stringify(stderr.toString())}`);
  } finally {
    child.stdout?.off('data', out); child.stderr?.off('data', err); child.off('error', error);
    // Continue draining a successful child's pipes without retaining unbounded data.
    child.stdout?.resume(); child.stderr?.resume();
  }
}
