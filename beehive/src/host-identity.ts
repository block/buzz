import { createCredential, credentialReference, readCredential, systemCredentials, type CredentialBackend } from './credential-store.ts';
import { existsSync, mkdirSync, rmdirSync } from 'node:fs';
import { join } from 'node:path';
import { fields, newKey, object, publicKey } from './protocol.ts';
import { readPrivate, writePrivate } from './storage.ts';
import { hostPairing, verifyHostRegistration, type HostPairing, type HostRegistration } from './host-registration.ts';

/** Infrastructure registration is separate from relay admission and agent placement. */
export type HostIdentity = { version: 3; pairing: HostPairing; secret: string; registration: HostRegistration | null };
function locked<T>(directory: string, operation: () => T): T {
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  const lock = join(directory, 'host.lock');
  mkdirSync(lock, { mode: 0o700 });
  try { return operation(); } finally { rmdirSync(lock); }
}
/** Offline bootstrap; neither requires admission nor copies the owner's secret. */
export function bootstrapHostIdentity(directory: string, label: string, owner: string, relay: string, credentials: CredentialBackend = systemCredentials): HostIdentity {
  return locked(directory, () => {
    if (existsSync(join(directory, 'setup.json'))) throw Error('Legacy installation retained; use a clean directory');
    if (existsSync(join(directory, 'host-identity.json'))) throw Error('Host identity already exists; no replacement or automatic migration');
    const secret = newKey();
    const pairing = hostPairing({ version: 1, purpose: 'beehive-host-registration', host: publicKey(secret), owner, label, relay, nonce: newKey() });
    const key = createCredential('host', secret, credentials);
    const identity: HostIdentity = { version: 3, pairing, secret, registration: null };
    writePrivate(join(directory, 'host-identity.json'), { version: 3, pairing, key, registration: null }, true);
    return identity;
  });
}
/** Reject legacy broad OA records rather than silently migrating their authority. */
export function readHostIdentity(directory: string, credentials: CredentialBackend = systemCredentials): HostIdentity {
  const value = readHostIdentityPublic(directory);
  return { ...value, secret: readCredential(credentialReference('host', value.pairing.host), credentials) };
}
/** Read only Beehive-owned public enrollment state; exporting never needs credential access. */
export function readHostIdentityPublic(directory: string): Omit<HostIdentity, 'secret'> {
  const value = object(readPrivate(join(directory, 'host-identity.json')));
  fields(value, ['version', 'pairing', 'key', 'registration']);
  if (value.version !== 3) throw Error('Legacy identity retained; explicit consent required for migration');
  const pairing = hostPairing(value.pairing);
  if (JSON.stringify(value.key) !== JSON.stringify(credentialReference('host', pairing.host))) throw Error('Invalid host credential reference');
  return { version: 3, pairing, registration: value.registration as HostRegistration | null };
}
/** Local configuration is the management trust root. Retain legacy approval as
 * public history only. Live startup uses the operator-approved bounded helper. */
export async function readHostIdentityAsync(directory: string, credentials: CredentialBackend, signal?: AbortSignal): Promise<HostIdentity> {
  signal?.throwIfAborted();
  const value = object(readPrivate(join(directory, 'host-identity.json')));
  fields(value, ['version', 'pairing', 'key', 'registration']);
  if (value.version !== 3) throw Error('Legacy identity retained; explicit consent required for migration');
  const pairing = hostPairing(value.pairing);
  const key = credentialReference('host', pairing.host);
  if (JSON.stringify(value.key) !== JSON.stringify(key)) throw Error('Invalid host credential reference');
  if (!credentials.readAsync) throw Error('Bounded asynchronous host credential reader required');
  const secret = await credentials.readAsync(key, signal);
  signal?.throwIfAborted();
  if (secret === null) throw Error('Local host key missing; explicit matching import required');
  if (publicKey(secret) !== pairing.host) throw Error('Credential identity mismatch');
  return { version: 3, pairing, secret, registration: value.registration as HostRegistration | null };
}
/** Atomically import only an owner registration matching the exact retained request. */
export function enrollHostIdentity(directory: string, registration: unknown, now = Math.floor(Date.now() / 1000), credentials: CredentialBackend = systemCredentials): void {
  locked(directory, () => {
    const identity = readHostIdentity(directory, credentials);
    identity.registration = verifyHostRegistration(registration, identity.pairing, now);
    writePrivate(join(directory, 'host-identity.json'), { version: 3, pairing: identity.pairing, key: credentialReference('host', identity.pairing.host), registration: identity.registration });
  });
}
