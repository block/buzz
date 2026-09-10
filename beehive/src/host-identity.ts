import { existsSync, mkdirSync, rmdirSync } from 'node:fs';
import { join } from 'node:path';
import { fields, newKey, object, publicKey, text } from './protocol.ts';
import { readPrivate, writePrivate } from './storage.ts';
import { verifyHostAttestation, type HostAttestation } from './host-attestation.ts';

/** Independent management identity. Agent slots and assignment authority are separate. */
export type HostIdentity = { version: 1; label: string; owner: string; secret: string; attestation: HostAttestation | null };
function locked<T>(directory: string, operation: () => T): T {
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  const lock = join(directory, 'host.lock');
  mkdirSync(lock, { mode: 0o700 });
  try { return operation(); } finally { rmdirSync(lock); }
}
/** Owner-public-only bootstrap. Pending until genuine owner attestation import; never migrates legacy keys. */
export function bootstrapHostIdentity(directory: string, label: string, owner: string): HostIdentity {
  text(label);
  if (!/^[0-9a-f]{64}$/.test(owner)) throw Error('Expected owner public key');
  return locked(directory, () => {
    if (existsSync(join(directory, 'setup.json'))) throw Error('Legacy installation retained; use a clean directory');
    const identity: HostIdentity = { version: 1, label, owner, secret: newKey(), attestation: null };
    writePrivate(join(directory, 'host-identity.json'), identity, true);
    return identity;
  });
}
/** Read local identity without converting absent/expired enrollment into trust. */
export function readHostIdentity(directory: string): HostIdentity {
  const value = object(readPrivate(join(directory, 'host-identity.json')));
  fields(value, ['version', 'label', 'owner', 'secret', 'attestation']);
  if (value.version !== 1 || typeof value.owner !== 'string' || !/^[0-9a-f]{64}$/.test(value.owner) || typeof value.secret !== 'string' || !/^[0-9a-f]{64}$/.test(value.secret)) throw Error('Invalid host identity');
  text(value.label); publicKey(value.secret);
  return value as HostIdentity;
}
/** Atomically import genuine authorization under the same installation lock as lifecycle setup. */
export function enrollHostIdentity(directory: string, attestation: unknown, now = Math.floor(Date.now() / 1000)): void {
  locked(directory, () => {
    const identity = readHostIdentity(directory);
    identity.attestation = verifyHostAttestation(attestation, publicKey(identity.secret), identity.owner, now);
    writePrivate(join(directory, 'host-identity.json'), identity);
  });
}
/** Admission prerequisite, not a substitute for message/assignment authorization. */
export function requireHostEnrollment(identity: HostIdentity, now = Math.floor(Date.now() / 1000)): HostAttestation {
  return verifyHostAttestation(identity.attestation, publicKey(identity.secret), identity.owner, now);
}
