import { mkdir, open, rename, unlink } from 'node:fs/promises';
import { dirname } from 'node:path';
import { randomUUID } from 'node:crypto';

/** Fixture fault/barrier boundary; production never supplies a hook. */
export type SnapshotBoundary = (phase: 'write' | 'rename' | 'directory') => Promise<void>;
/** Replacement occurred but durability is uncertain: stop this writer until reopen. */
export class UncertainSnapshot extends Error {}

/** Atomic owner-only snapshot; resolves only after file AND directory fsync. */
export async function writeRelaySnapshot(path: string, value: unknown, boundary?: SnapshotBoundary): Promise<void> {
  await mkdir(dirname(path), { recursive: true, mode: 0o700 });
  const temp = `${path}.${randomUUID()}.tmp`;
  let replaced = false;
  try {
    const file = await open(temp, 'wx', 0o600);
    try {
      await boundary?.('write');
      await file.writeFile(JSON.stringify(value));
      await file.sync();
    } finally { await file.close(); }
    await boundary?.('rename');
    await rename(temp, path);
    replaced = true;
    const dir = await open(dirname(path), 'r');
    try { await boundary?.('directory'); await dir.sync(); }
    finally { await dir.close(); }
  } catch (error) {
    if (replaced) throw new UncertainSnapshot('Relay snapshot replacement durability uncertain', { cause: error });
    throw error;
  } finally {
    if (!replaced) await unlink(temp).catch(error => { if (error.code !== 'ENOENT') throw error; });
  }
}
