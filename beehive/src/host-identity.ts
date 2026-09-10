import { existsSync, mkdirSync, rmdirSync } from 'node:fs';
import { join } from 'node:path';
import { fields, newKey, object, publicKey } from './protocol.ts';
import { readPrivate, writePrivate } from './storage.ts';
import { hostPairing, verifyHostRegistration, type HostPairing, type HostRegistration } from './host-registration.ts';

/** Infrastructure registration is separate from relay admission and agent placement. */
export type HostIdentity = { version: 2; pairing: HostPairing; secret: string; registration: HostRegistration | null };
function locked<T>(directory: string, operation: () => T): T {
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  const lock = join(directory, 'host.lock');
  mkdirSync(lock, { mode: 0o700 });
  try { return operation(); } finally { rmdirSync(lock); }
}
/** Offline bootstrap; neither requires admission nor copies the owner's secret. */
export function bootstrapHostIdentity(directory: string, label: string, owner: string, relay: string): HostIdentity {
  return locked(directory, () => {
    if (existsSync(join(directory, 'setup.json'))) throw Error('Legacy installation retained; use a clean directory');
    const secret = newKey();
    const pairing = hostPairing({ version: 1, purpose: 'beehive-host-registration', host: publicKey(secret), owner, label, relay, nonce: newKey() });
    const identity: HostIdentity = { version: 2, pairing, secret, registration: null };
    writePrivate(join(directory, 'host-identity.json'), identity, true);
    return identity;
  });
}
/** Reject legacy broad OA records rather than silently migrating their authority. */
export function readHostIdentity(directory: string): HostIdentity {
  const value = object(readPrivate(join(directory, 'host-identity.json')));
  fields(value, ['version', 'pairing', 'secret', 'registration']);
  if (value.version !== 2 || typeof value.secret !== 'string' || !/^[0-9a-f]{64}$/.test(value.secret)) throw Error('Invalid host identity; legacy OA is not infrastructure enrollment');
  const pairing = hostPairing(value.pairing);
  if (publicKey(value.secret) !== pairing.host) throw Error('Host key differs from pairing request');
  return { version: 2, pairing, secret: value.secret, registration: value.registration as HostRegistration | null };
}
/** Atomically import only an owner registration matching the exact retained request. */
export function enrollHostIdentity(directory: string, registration: unknown, now = Math.floor(Date.now() / 1000)): void {
  locked(directory, () => {
    const identity = readHostIdentity(directory);
    identity.registration = verifyHostRegistration(registration, identity.pairing, now);
    writePrivate(join(directory, 'host-identity.json'), identity);
  });
}
/** Valid registration is NOT relay admission. Remains no-network until a narrow contract exists. */
export function requireHostEnrollment(identity: HostIdentity, now = Math.floor(Date.now() / 1000)): never {
  verifyHostRegistration(identity.registration, identity.pairing, now);
  throw Error('Host registered locally; relay admission pending: broad NIP-OA owner delegation is prohibited');
}
