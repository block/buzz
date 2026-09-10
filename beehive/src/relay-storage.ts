import { trace } from './latency-trace.ts';
import { mkdir, open, rename, unlink } from 'node:fs/promises';
import { dirname } from 'node:path';
import { randomUUID } from 'node:crypto';

/** Fixture fault/barrier boundary; production never supplies a hook. */
export type SnapshotBoundary = (phase: 'write' | 'rename' | 'directory') => Promise<void>;
/** Replacement occurred but durability is uncertain: stop this writer until reopen. */
export class UncertainSnapshot extends Error {}

/** Atomic owner-only snapshot; resolves only after file AND directory fsync. */
export async function writeRelaySnapshot(path: string, value: unknown, boundary?: SnapshotBoundary): Promise<void> {
  trace('relay.io.begin', 'mkdir');
  await mkdir(dirname(path), { recursive: true, mode: 0o700 });
  trace('relay.io.end', 'mkdir');
  const temp = `${path}.${randomUUID()}.tmp`;
  let replaced = false;
  try {
    trace('relay.io.begin', 'file.open');
    const file = await open(temp, 'wx', 0o600);
    trace('relay.io.end', 'file.open');
    try {
      await boundary?.('write');
      trace('relay.io.begin', 'file.write');
      await file.writeFile(JSON.stringify(value));
      trace('relay.io.end', 'file.write');
      trace('relay.io.begin', 'file.sync');
      await file.sync();
      trace('relay.io.end', 'file.sync');
    } finally { trace('relay.io.begin', 'file.close'); await file.close(); trace('relay.io.end', 'file.close'); }
    await boundary?.('rename');
    trace('relay.io.begin', 'rename');
    await rename(temp, path);
    trace('relay.io.end', 'rename');
    replaced = true;
    trace('relay.io.begin', 'dir.open');
    const dir = await open(dirname(path), 'r');
    trace('relay.io.end', 'dir.open');
    try { await boundary?.('directory'); trace('relay.io.begin', 'dir.sync'); await dir.sync(); trace('relay.io.end', 'dir.sync'); }
    finally { trace('relay.io.begin', 'dir.close'); await dir.close(); trace('relay.io.end', 'dir.close'); }
  } catch (error) {
    if (replaced) throw new UncertainSnapshot('Relay snapshot replacement durability uncertain', { cause: error });
    throw error;
  } finally {
    if (!replaced) await unlink(temp).catch(error => { if (error.code !== 'ENOENT') throw error; });
  }
}
