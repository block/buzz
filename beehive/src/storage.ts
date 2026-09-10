import { closeSync, fsyncSync, mkdirSync, openSync, readFileSync, renameSync, writeFileSync, lstatSync, linkSync, unlinkSync } from 'node:fs';
import { dirname } from 'node:path';
import { randomUUID } from 'node:crypto';
/** Private atomic replacement, including file/directory fsync. Parent must be operator-owned. */
export function writePrivate(path: string, value: unknown, exclusive = false): void {
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  const temp = `${path}.${randomUUID()}.tmp`;
  const fd = openSync(temp, 'wx', 0o600);
  try { writeFileSync(fd, JSON.stringify(value)); fsyncSync(fd); } finally { closeSync(fd); }
  if (exclusive) {
    try { linkSync(temp,path); } finally { unlinkSync(temp); }
  } else renameSync(temp,path);
  const dir = openSync(dirname(path),'r');
  try { fsyncSync(dir); } finally { closeSync(dir); }
}
export function readPrivate(path: string): unknown {
  const stat = lstatSync(path);
  if (!stat.isFile() || (stat.mode & 0o077)) throw Error('Local file must be regular and owner-only');
  return JSON.parse(readFileSync(path,'utf8'));
}
