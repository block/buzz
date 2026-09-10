import { createRequire } from 'node:module';
import type { CredentialBackend, CredentialReference } from './credential-store.ts';

/** Narrow native API; never enumerate credentials or select another keychain target. */
export type NativeEntry = { getPassword(): string | null; setPassword(secret: string): void; deleteCredential(): boolean };
type EntryConstructor = new (service: string, account: string) => NativeEntry;
const require = createRequire(import.meta.url);

/** Load the pinned prebuilt bridge only. This does not test OS availability and does
 * not create an Entry. The bridge cannot suppress OS approval/unlock dialogs. */
export function loadNativeEntry(): EntryConstructor {
  if (process.env.NAPI_RS_NATIVE_LIBRARY_PATH || process.env.NAPI_RS_FORCE_WASI) throw Error('Beehive refuses native credential-library environment overrides');
  return (require('@napi-rs/keyring') as { Entry: EntryConstructor }).Entry;
}

/** Per-identity OS entries avoid shared-blob lost updates. Explicit create/import
 * and removal are serialized by their installation lock; no secret cache/fallback.
 * Native errors are intentionally not reclassified from unstable display strings.
 * Missing is null; locked, denied and unavailable remain actionable failures. */
export function nativeCredentials(load: () => EntryConstructor = loadNativeEntry): CredentialBackend {
  function entry(reference: CredentialReference): NativeEntry {
    if (reference.service !== 'beehive' || !['host', 'agent', 'owner'].includes(reference.role) || !/^[0-9a-f]{64}$/.test(reference.publicKey)) throw Error('Invalid Beehive credential reference');
    return new (load())('beehive', `${reference.role}:${reference.publicKey}`);
  }
  function operation<T>(action: () => T): T {
    try { return action(); } catch {
      // Do not include third-party error text: it may contain credential data.
      throw Error('Beehive OS credential operation failed (locked, denied, unavailable, or invalid entry). Check OS credential-store access explicitly and retry; no plaintext fallback or identity regeneration.');
    }
  }
  return {
    read: reference => operation(() => entry(reference).getPassword()),
    create: (reference, secret) => operation(() => {
      const item = entry(reference);
      if (item.getPassword() !== null) throw Error('Credential already exists; refusing replacement');
      item.setPassword(secret);
    }),
    remove: reference => operation(() => { entry(reference).deleteCredential(); }),
  };
}
