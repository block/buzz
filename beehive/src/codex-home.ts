import { createHash } from 'node:crypto';
import { lstatSync, mkdirSync, readFileSync, realpathSync, writeFileSync } from 'node:fs';
import { dirname, isAbsolute, join } from 'node:path';

/** Public, stable slot/runtime identity. Never contains a credential or session token. */
export type ManagedCodexHome = { parent: string; identity: string };
/** Pure projection: catalog/Save cannot create a directory or inspect user config. */
export function codexHomePaths(value: ManagedCodexHome) {
  if (!isAbsolute(value.parent) || !/^[0-9a-f]{64}$/.test(value.identity)) throw Error('Invalid managed Codex home');
  const root = join(value.parent, `beehive-codex-${value.identity}`);
  return { root, home: join(root, 'service-home'), config: join(root, 'agent-config') };
}
/** Bind isolation to the installation, public slot and immutable runtime ID. */
export function codexHomeIdentity(host: string, owner: string, agent: string, runtime: string) {
  if (![host, owner, agent].every(v => /^[0-9a-f]{64}$/.test(v))) throw Error('Managed Codex home requires public slot authority');
  return createHash('sha256').update(JSON.stringify([host, owner, agent, runtime])).digest('hex');
}
function directory(path: string) {
  const st = lstatSync(path);
  if (!st.isDirectory() || st.isSymbolicLink() || st.uid !== process.getuid?.() || (st.mode & 0o077) || realpathSync(path) !== path) throw Error('Unsafe managed Codex directory');
}
/** Admission-only provisioning. Never retarget legacy HOME, import config, or
 * delete retained state. Existing roots need an exact owner-only marker; partial
 * initialization fails closed rather than adopting an unrelated directory.
 * Persistent lifetime preserves state across Stop/reopen and future resume use. */
export function prepareCodexHome(value: ManagedCodexHome, home: string, config: string) {
  const paths = codexHomePaths(value);
  if (home !== paths.home || config !== paths.config) throw Error('Managed Codex home binding mismatch');
  directory(dirname(paths.root));
  let created = false;
  try { mkdirSync(paths.root, { mode: 0o700 }); created = true; }
  catch (error) { if ((error as NodeJS.ErrnoException).code !== 'EEXIST') throw error; }
  directory(paths.root);
  const marker = join(paths.root, 'beehive-owner.json');
  const expected = JSON.stringify({ version: 1, identity: value.identity });
  if (created) writeFileSync(marker, expected, { mode: 0o600, flag: 'wx' });
  const st = lstatSync(marker);
  if (!st.isFile() || st.isSymbolicLink() || st.uid !== process.getuid?.() || (st.mode & 0o077) || st.size !== Buffer.byteLength(expected) || readFileSync(marker, 'utf8') !== expected) throw Error('Managed Codex home ownership unavailable');
  for (const path of [home, config]) {
    try { mkdirSync(path, { mode: 0o700 }); }
    catch (error) { if ((error as NodeJS.ErrnoException).code !== 'EEXIST') throw error; }
    directory(path);
  }
}
