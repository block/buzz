import { existsSync } from 'node:fs';
import { readPrivate, writePrivate } from '../src/storage.ts';
import type { CredentialBackend } from '../src/credential-store.ts';
/** Fresh fixture secrets ONLY. Explicitly not OS storage, never imported by production. */
export function isolatedFileCredentials(file: string): CredentialBackend {
  if (!file || !file.includes('beehive-pairing-cli-')) throw Error('Missing isolated credential fixture path');
  const read = (): Record<string, string> => existsSync(file) ? readPrivate(file) as Record<string, string> : {};
  return {
    read(r) { return read()[JSON.stringify(r)] ?? null; },
    create(r, secret) { const data = read(), id = JSON.stringify(r); if (id in data) throw Error('Existing fixture key'); data[id] = secret; writePrivate(file, data); },
    remove(r) { const data = read(); delete data[JSON.stringify(r)]; writePrivate(file, data); },
  };
}
