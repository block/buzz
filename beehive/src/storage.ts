import { trace, traceSync } from './latency-trace.ts';
import { closeSync, fsyncSync, mkdirSync, openSync, readFileSync, renameSync, writeFileSync, lstatSync, linkSync, unlinkSync } from 'node:fs';
import { dirname } from 'node:path';
import { randomUUID } from 'node:crypto';
/** Private atomic replacement, including file/directory fsync. Parent must be operator-owned. */
export function writePrivate(path: string, value: unknown, exclusive = false): void {
  trace('private.begin', { file: path.split('/').slice(-1)[0] }); traceSync('mkdir-mode0700', () => mkdirSync(dirname(path), { recursive: true, mode: 0o700 }));
  const temp = `${path}.${randomUUID()}.tmp`;
  const fd = traceSync('open-exclusive-mode0600', () => openSync(temp, 'wx', 0o600));
  try { const json = traceSync('json', () => JSON.stringify(value)); const bytes = traceSync('utf8', () => Buffer.from(json)); traceSync('write', () => writeFileSync(fd, bytes)); trace('private.file-sync.begin'); fsyncSync(fd); trace('private.file-sync.end'); } finally { traceSync('close-file', () => closeSync(fd)); }
  if (exclusive) {
    try { traceSync('link', () => linkSync(temp,path)); } finally { traceSync('unlink', () => unlinkSync(temp)); }
  } else traceSync('rename', () => renameSync(temp,path));
  const dir = traceSync('open-directory', () => openSync(dirname(path),'r'));
  try { trace('private.dir-sync.begin'); fsyncSync(dir); trace('private.dir-sync.end'); } finally { traceSync('close-directory', () => closeSync(dir)); } trace('private.end');
}
export function readPrivate(path: string): unknown {
  const stat = lstatSync(path);
  if (!stat.isFile() || (stat.mode & 0o077)) throw Error('Local file must be regular and owner-only');
  return JSON.parse(readFileSync(path,'utf8'));
}
