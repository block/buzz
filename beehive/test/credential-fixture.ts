import type { CredentialBackend, CredentialReference } from '../src/credential-store.ts';
/** Explicit in-memory test backend, NOT an OS credential-store validation. */
export function memoryCredentials(): CredentialBackend {
  const values = new Map<string, string>();
  const id = (r: CredentialReference) => JSON.stringify(r);
  return {
    read: r => values.get(id(r)) ?? null,
    create(r, secret) { if (values.has(id(r))) throw Error('Already exists'); values.set(id(r), secret); },
    remove(r) { values.delete(id(r)); },
  };
}
